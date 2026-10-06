"""FFI record fields that no production code writes.

A field on a `uniffi::Record` that nothing sets still reads as data: Swift
renders it, the CLI prints it, and every reader sees the same `None`, `0` or
empty string. rustc cannot say so — the field is `pub` in a library, so
`dead_code` treats it as API — and a source scan cannot either: a write is a
struct literal spread over twenty lines, `..Default::default()` leaving the
field out, `record.field = value`, `&mut record.field` handed to a method or
a closure, and which struct `x.field` names depends on the type of `x`.

So this reads the compiler's MIR (`rustc --emit=mir`, stable). Every place
there carries its type, a struct literal is an aggregate that names each
field, and `#[cfg(test)]` code is already compiled out. A field counts as
written when production code

  * builds the record with a value for it that is not a default — `None`,
    `false`, `0`, an empty string or collection, or `Default::default()` of a
    std type,
  * assigns the field such a value, or assigns anything inside it,
  * borrows the field, or anything inside it, mutably, since whatever takes
    the borrow may write through it,
  * copies it from another record's field that is itself written, or
  * is hand-written Swift or Kotlin outside the tests that constructs the
    record, which has to supply every field.

Not writes: a record's derived impls (`Clone`, `Default`, `Deserialize`,
UniFFI's lifting of a record a front end sent), a struct literal's `..base`,
and copying a field out of another record of the same type. Deserializing
puts back what serializing took out, so it originates nothing — except for a
record that derives `Deserialize` without `Serialize`, which core never
writes out and so can only read from outside: a bundled catalog or a
provider's response. Every field of such a record counts as written.
"""

import pathlib
import re
import sys
from collections import defaultdict

# --- MIR places ---------------------------------------------------------------
#
# rustc prints a place's projections as wrappers around its local: `(*_1)`
# is a deref, `(_1.3: T)` field 3 with type T, `(_1 as variant#2)` a downcast
# and `_1[_2]` an index. They nest: `(((*_1).3: Inner).0: u64)`.


class Place:
    __slots__ = ('kind', 'inner', 'local', 'index', 'type')

    def __init__(self, kind, inner=None, local=None, index=None, type_=None):
        self.kind, self.inner, self.local = kind, inner, local
        self.index, self.type = index, type_

    def chain(self):
        """Each projection from the outermost in, ending at the local."""
        node = self
        while node is not None:
            yield node
            node = node.inner

    def root(self):
        *_, local = self.chain()
        return local.local


def skip_balanced(text, i, stop):
    """Index of the first `stop` outside brackets, from `i`.

    Angle brackets are not counted, because `fn(A) -> B` prints a `>` with no
    `<`. Parentheses, brackets and braces always balance in a printed type.
    """
    depth = 0
    while i < len(text):
        c = text[i]
        if depth == 0 and c == stop:
            return i
        if c in '([{':
            depth += 1
        elif c in ')]}':
            depth -= 1
            if depth < 0:
                return i
        i += 1
    return i


def parse_place(text, i=0):
    """(Place, index past it), or (None, i) when no place starts at `i`."""
    if text.startswith('(*', i):
        inner, j = parse_place(text, i + 2)
        if inner is None or not text.startswith(')', j):
            return None, i
        place, j = Place('deref', inner), j + 1
    elif text.startswith('(', i):
        inner, j = parse_place(text, i + 1)
        if inner is None:
            return None, i
        field = re.compile(r'\.(\d+): ').match(text, j)
        if field:
            end = skip_balanced(text, field.end(), ')')
            place = Place('field', inner, index=int(field.group(1)),
                          type_=text[field.end():end])
        elif text.startswith(' as ', j):
            end = skip_balanced(text, j, ')')
            place = Place('downcast', inner)
        else:
            return None, i
        j = end + 1
    else:
        local = re.compile(r'_(\d+)\b').match(text, i)
        if not local:
            return None, i
        place, j = Place('local', local=int(local.group(1))), local.end()
    while text.startswith('[', j):
        place, j = Place('index', place), skip_balanced(text, j + 1, ']') + 1
    return place, j


def deref_type(type_):
    type_ = re.sub(r"^(?:&(?:'\w+ )?(?:mut )?|\*(?:mut|const) )", '', type_)
    boxed = re.match(r'^(?:std::boxed::|alloc::boxed::)?Box<(.*)>$', type_)
    return boxed.group(1) if boxed else type_


def type_name(type_):
    """`store::state::ResidentState<'_>` → `ResidentState`."""
    return type_.split('<', 1)[0].strip().rsplit('::', 1)[-1]


def place_type(place, local_types):
    if place.kind == 'local':
        return local_types.get(place.local, '')
    if place.kind == 'deref':
        return deref_type(place_type(place.inner, local_types))
    if place.kind == 'field':
        return place.type
    if place.kind == 'downcast':
        return place_type(place.inner, local_types)
    element = re.match(r'^\[(.*?)(?:; \d+)?\]$', place_type(place.inner, local_types))
    return element.group(1) if element else ''


# --- MIR bodies ---------------------------------------------------------------

HEADER = re.compile(r'^(?:fn|const|static) ')
LOCAL = re.compile(r'^\s+let (?:mut )?_(\d+): (.*);$')
STATEMENT_INDENT = ' ' * 8


class Body:
    """One function, closure, constant or static: its local types and
    the statements of its basic blocks."""

    def __init__(self, header):
        self.header = header
        self.locals = parameters(header)
        self.statements = []


def split_top_level(text):
    """Split at `, ` outside brackets, braces and generic arguments."""
    parts, depth, start = [], 0, 0
    for i, c in enumerate(text):
        if c in '([{<':
            depth += 1
        elif c in ')]}' or (c == '>' and text[i - 1:i] != '-'):
            depth -= 1
        elif depth == 0 and text.startswith(', ', i):
            parts.append(text[start:i])
            start = i + 2
    parts.append(text[start:])
    return [part for part in parts if part]


def parameters(header):
    """{1: '&mut Rec'} from `fn name(_1: &mut Rec) -> R {`."""
    if not header.startswith('fn '):
        return {}
    open_ = skip_balanced(header, 3, '(')
    close = skip_balanced(header, open_ + 1, ')')
    found = {}
    for part in split_top_level(header[open_ + 1:close]):
        parameter = re.match(r'^_(\d+): (.*)$', part)
        if parameter:
            found[int(parameter.group(1))] = parameter.group(2)
    return found


def bodies(lines):
    body = None
    for line in lines:
        line = line.rstrip('\n')
        if HEADER.match(line):
            body = Body(line)
        elif body is None:
            continue
        elif line == '}':
            yield body
            body = None
        elif local := LOCAL.match(line):
            body.locals[int(local.group(1))] = local.group(2)
        elif line.startswith(STATEMENT_INDENT) and line[len(STATEMENT_INDENT)] != ' ':
            body.statements.append(line.strip())


CALL_TARGET = re.compile(r' -> (?:\[[^\[\]]*\]|bb\d+|unwind \w+)$')


def statement_parts(statement):
    """(assigned place, rvalue) for an assignment, or (None, statement)."""
    place, end = parse_place(statement)
    if place is None or not statement.startswith(' = ', end):
        return None, statement
    rvalue = statement[end + 3:].removesuffix(';')
    # A call names where it returns: `f(move _2) -> [return: bb1, …]`.
    target = CALL_TARGET.search(rvalue)
    return place, rvalue[:target.start()] if target else rvalue


# --- Values -------------------------------------------------------------------

DEFAULT_CONST = re.compile(
    r'^const (?:false|0(?:_[iu](?:8|16|32|64|128|size))?|0(?:\.0)?(?:_?f(?:32|64))?|""'
    r'|(?:[\w:]+::)?Option::<.*>::None)$')
NONE = re.compile(r'^(?:[\w:]+::)?Option::<.*>::None$')
EMPTY = re.compile(r'^(?:[\w:]+::)?(?:String|Vec|VecDeque|HashMap|HashSet|BTreeMap|BTreeSet)'
                   r'(?:::<.*>)?::new\(\)$')
DEFAULT_CALL = re.compile(r'^<(.*) as (?:std::default::)?Default>::default\(\)$')
STD_DEFAULTS = {'Option', 'String', 'Vec', 'VecDeque', 'HashMap', 'HashSet', 'BTreeMap',
                'BTreeSet', 'bool', 'u8', 'u16', 'u32', 'u64', 'u128', 'usize', 'i8',
                'i16', 'i32', 'i64', 'i128', 'isize', 'f32', 'f64'}
CONVERSION = re.compile(r'^<.* as [\w:]*(?:ToString|ToOwned|From|Into)(?:<.*>)?>::'
                        r'(?:to_string|to_owned|from|into)\(((?:move|copy|const) [^,]*)\)$')
CLONE = re.compile(r'^<.* as (?:std::clone::)?Clone>::clone\((?:move|copy) _(\d+)\)$')
MUT_BORROW = re.compile(r'&(?:raw )?mut ')

# A value is WRITE, or the record fields it was copied from: an empty set
# when every way it is produced is a default.
WRITE = 'write'


class Values:
    """Where the values in one body come from."""

    def __init__(self, body, records):
        self.body, self.records = body, records
        self.definitions = defaultdict(list)
        # A local built empty and then filled — `let mut v = Vec::new();
        # v.push(x)` — changes through a borrow, not by being assigned again.
        self.changed = set()
        for statement in body.statements:
            place, rvalue = statement_parts(statement)
            if place is not None and place.kind == 'local':
                self.definitions[place.local].append(rvalue)
            elif place is not None:
                self.changed.add(place.root())
            for borrow in MUT_BORROW.finditer(rvalue):
                borrowed, _ = parse_place(rvalue, borrow.end())
                if borrowed is not None:
                    self.changed.add(borrowed.root())

    def record_field(self, place):
        if place is not None and place.kind == 'field':
            name = type_name(place_type(place.inner, self.body.locals))
            if name in self.records:
                return frozenset({(name, place.index)})
        return None

    def operand(self, text, seen=frozenset()):
        if text.startswith('const '):
            return frozenset() if DEFAULT_CONST.match(text) else WRITE
        moved = re.match(r'^(?:move|copy) (.*)$', text)
        if not moved:
            return WRITE
        place, end = parse_place(moved.group(1))
        if place is None or end != len(moved.group(1)):
            return WRITE
        if place.kind == 'local':
            return self.local(place.local, seen)
        return self.record_field(place) or WRITE

    def local(self, local, seen):
        if local in seen:
            return frozenset()
        if local in self.changed or not self.definitions.get(local):
            return WRITE
        sources = set()
        for rvalue in self.definitions[local]:
            value = self.rvalue(rvalue, seen | {local})
            if value == WRITE:
                return WRITE
            sources |= value
        return frozenset(sources)

    def rvalue(self, text, seen=frozenset()):
        if text.startswith(('move ', 'copy ', 'const ')):
            return self.operand(text, seen)
        if NONE.match(text) or EMPTY.match(text):
            return frozenset()
        if default := DEFAULT_CALL.match(text):
            return frozenset() if type_name(default.group(1)) in STD_DEFAULTS else WRITE
        if conversion := CONVERSION.match(text):
            return self.operand(conversion.group(1), seen)
        if clone := CLONE.match(text):
            # `record.field.clone()` clones a borrow of the field.
            for borrow in self.definitions.get(int(clone.group(1)), []):
                if borrow.startswith('&') and not MUT_BORROW.match(borrow):
                    field = self.record_field(parse_place(borrow[1:])[0])
                    if field:
                        return field
        return WRITE


# --- Writes -------------------------------------------------------------------

AGGREGATE = re.compile(r'^([\w:]+?)(?:::<.*?>)? \{ (.*) \}$')


def aggregate(rvalue, records):
    """(record, [(field, operand)]) when `rvalue` is a struct literal of one.

    `Enum::Variant { … }` is not: rustc's naming lints keep modules lower
    case, so a capitalised parent is an enum.
    """
    literal = AGGREGATE.match(rvalue)
    if not literal:
        return None, []
    *parents, name = literal.group(1).split('::')
    if name not in records or (parents and parents[-1][:1].isupper()):
        return None, []
    fields = []
    for part in split_top_level(literal.group(2)):
        field = re.match(r'^(?:r#)?(\w+): (.*)$', part)
        if field:
            fields.append((field.group(1), field.group(2)))
    return name, fields


def record_fields_in(place, local_types, records):
    """(record, field index, projection) for each record field the place
    lies within, outermost first."""
    found = []
    for node in place.chain():
        if node.kind == 'field':
            name = type_name(place_type(node.inner, local_types))
            if name in records:
                found.append((name, node.index, node))
    return found


def body_writes(body, records):
    """(record, field index, value) for each write in a body."""
    values = Values(body, records)
    found = []
    for statement in body.statements:
        place, rvalue = statement_parts(statement)
        if place is not None:
            # Assigning the field a default is no write; assigning anything
            # inside it changes it.
            for record, index, node in record_fields_in(place, body.locals, records):
                found.append((record, index, values.rvalue(rvalue) if node is place else WRITE))
            record, fields = aggregate(rvalue, records)
            for index, (_, operand) in enumerate(fields):
                value = values.operand(operand)
                if value != frozenset({(record, index)}):  # not `..base`
                    found.append((record, index, value))
        for borrow in MUT_BORROW.finditer(rvalue):
            borrowed, _ = parse_place(rvalue, borrow.end())
            if borrowed is not None:
                for record, index, _ in record_fields_in(borrowed, body.locals, records):
                    found.append((record, index, WRITE))
    return found


def field_names(body, records):
    """{record: field names in declaration order}, from its struct literals."""
    names = {}
    for statement in body.statements:
        if ' { ' in statement:
            record, fields = aggregate(statement_parts(statement)[1], records)
            if record:
                names.setdefault(record, [name for name, _ in fields])
    return names


# --- Generated code -----------------------------------------------------------

IMPL_AT = re.compile(r'<impl at ([^:>]+):(\d+):(\d+): (\d+):(\d+)>')


class Derives:
    """Which bodies a `#[derive(…)]` generated.

    rustc names a derived impl after the span of the trait in the attribute,
    `<impl at core/src/lib.rs:21:17: 21:22>`; a written impl's span covers
    the whole impl block.
    """

    def __init__(self, root):
        self.root = pathlib.Path(root)
        self.files = {}

    def lines(self, path):
        if path not in self.files:
            file = self.root / path
            self.files[path] = file.read_text().splitlines() if file.is_file() else []
        return self.files[path]

    def derived(self, header):
        for impl in IMPL_AT.finditer(header):
            path, line, _, end_line, _ = impl.groups()
            lines = self.lines(path)
            if line != end_line or not 0 < int(line) <= len(lines):
                continue
            # rustfmt may have put the trait on a line of its own inside the
            # attribute: look back to where the attribute opens.
            start = int(line) - 1
            while start > 0 and not lines[start].lstrip().startswith('#['):
                if lines[start].rstrip().endswith((';', '{', '}')):
                    break
                start -= 1
            if lines[start].lstrip().startswith('#[derive('):
                return True
        return False


# --- Records ------------------------------------------------------------------

RECORD = re.compile(r'#\[derive\(([^\]]*)\)\]\s*(?:(?:#\[[^\]]*\]|//[^\n]*)\s*)*'
                    r'pub struct (\w+)')


class Record:
    def __init__(self, name, path, line, derives):
        self.name, self.path, self.line = name, path, line
        self.derives = {derive.strip().rsplit('::', 1)[-1] for derive in derives.split(',')}

    def read_from_outside(self):
        return 'Deserialize' in self.derives and 'Serialize' not in self.derives


def records_in(paths):
    """{name: Record} for every struct deriving `uniffi::Record`."""
    found = {}
    for path in paths:
        text = path.read_text()
        for record in RECORD.finditer(text):
            if re.search(r'\buniffi::Record\b', record.group(1)):
                line = text.count('\n', 0, record.start(2)) + 1
                found[record.group(2)] = Record(record.group(2), str(path), line,
                                                record.group(1))
    return found


def unwritten(writes, names, written):
    """Fields with no write once copies of written fields count."""
    written = set(written)
    copies = defaultdict(set)
    for record, index, value in writes:
        if value == WRITE:
            written.add((record, index))
        else:
            copies[(record, index)] |= value
    changed = True
    while changed:
        changed = False
        for field, sources in copies.items():
            if field not in written and sources & written:
                written.add(field)
                changed = True
    return [(record, name) for record, fields in sorted(names.items())
            for index, name in enumerate(fields) if (record, index) not in written]


def analyse(mir_files, records, root, front_ends):
    """(unwritten (record, field) pairs, records no MIR builds)."""
    derives = Derives(root)
    writes, names = [], {}
    for mir in mir_files:
        with open(mir, encoding='utf-8', errors='replace') as lines:
            for body in bodies(lines):
                for record, fields in field_names(body, records).items():
                    names.setdefault(record, fields)
                if not derives.derived(body.header):
                    writes.extend(body_writes(body, records))
    written = {(name, index) for name, fields in names.items()
               if records[name].read_from_outside()
               or re.search(rf'\b{name}\s*\(', front_ends)
               for index in range(len(fields))}
    return unwritten(writes, names, written), sorted(set(records) - set(names))


def main(mir_files):
    sys.path.insert(0, str(pathlib.Path(__file__).parent))
    from source_scan import frontend_test, hand_written

    records = records_in(sorted(pathlib.Path('core/src').rglob('*.rs')))
    front_ends = '\n'.join(path.read_text()
                           for root, suffix in (('swift', '.swift'), ('kotlin', '.kt'))
                           for path in hand_written(root, suffix)
                           if not frontend_test(path) and 'generated' not in path.parts)
    fields, unbuilt = analyse(mir_files, records, '.', front_ends)
    for record, field in fields:
        where = f'{records[record].path}:{records[record].line}'
        print(f'  {record + "." + field:<56} {where}')
    for record in unbuilt:
        print(f'  {record:<56} no struct literal in the MIR')
    print(f'\n  {len(fields)} unwritten record field(s), {len(unbuilt)} unbuilt record(s)')
    return 1 if fields or unbuilt else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
