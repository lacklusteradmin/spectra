// House style for icons/crypto and icons/fiat (see scripts/normalize-icons.sh).
//
// Every icon is one 64x64 viewBox with a full-bleed background disc and artwork
// on top. The two custom plugins below strip attributes that vector editors
// export but that draw nothing, and that svgo's own presets leave in place.

const DRAWS = new Set([
  'path', 'circle', 'ellipse', 'rect', 'line', 'polyline', 'polygon', 'text', 'use', 'image',
]);
const SKIP = new Set(['defs', 'clipPath', 'mask', 'symbol', 'linearGradient', 'radialGradient']);

// `fill="none"` on <svg> is a Figma export habit. It only paints when some
// descendant leaves fill unset, so drop it when every shape names its own fill.
const removeRootFillNone = {
  name: 'removeRootFillNone',
  fn: () => ({
    element: {
      enter: (node, parentNode) => {
        if (parentNode.type !== 'root' || node.attributes.fill !== 'none') return;
        const inherits = (n, filled) => {
          if (SKIP.has(n.name)) return false;
          const has = filled || n.attributes.fill !== undefined;
          if (DRAWS.has(n.name)) return !has;
          return (n.children ?? []).some((c) => c.type === 'element' && inherits(c, has));
        };
        if (!node.children.some((c) => c.type === 'element' && inherits(c, false))) {
          delete node.attributes.fill;
        }
      },
    },
  }),
};

// `clip-rule` only means anything inside a <clipPath>. Everywhere else it is
// noise that rides along with `fill-rule` out of the exporter.
const removeDeadClipRule = {
  name: 'removeDeadClipRule',
  fn: () => {
    let depth = 0;
    return {
      element: {
        enter: (node) => {
          if (node.name === 'clipPath') depth += 1;
          if (depth === 0) delete node.attributes['clip-rule'];
        },
        exit: (node) => {
          if (node.name === 'clipPath') depth -= 1;
        },
      },
    };
  },
};

// svgo drops `fill="#000"` because black is the SVG default. The disc under
// every icon should still say what colour it is, so put it back.
const explicitBackgroundFill = {
  name: 'explicitBackgroundFill',
  fn: () => ({
    element: {
      enter: (node, parentNode) => {
        if (parentNode.name !== 'svg' || parentNode.attributes.fill !== undefined) return;
        if (!DRAWS.has(node.name) || node.attributes.fill !== undefined) return;
        node.attributes.fill = '#000';
      },
    },
  }),
};

// An icon dropped in at its source size (512x512, 140x140, ...) still draws
// correctly, so nothing downstream complains — it just is not a member of the
// library. Rescale a square viewBox onto 0 0 64 64 and let convertPathData bake
// the factor into the coordinates. A viewBox that is not square, or does not
// start at 0 0, is left alone: fitting that artwork onto the disc is a design
// decision, not a rewrite.
// A shape whose geometry can be scaled in place, rather than by wrapping it in
// a transform. `stroke-width` rides along with the geometry: a transform scales
// the stroke as part of the matrix, so a shape rescaled attribute by attribute
// has to be told. Leaving it out made an icon's ring the source file's width at
// the library's size — 3x too thick on a 210px source, and clipped by the
// viewBox it now overflowed.
const STROKE = 'stroke-width';
const SCALES = { circle: ['cx', 'cy', 'r', STROKE] };
const scaleToLibraryViewBox = {
  name: 'scaleToLibraryViewBox',
  fn: () => ({
    element: {
      enter: (node, parentNode) => {
        if (parentNode.type !== 'root' || node.name !== 'svg') return;
        const box = /^0 0 (\d*\.?\d+) (\d*\.?\d+)$/.exec(node.attributes.viewBox ?? '');
        if (!box || box[1] !== box[2] || box[1] === '64') return;
        const k = 64 / Number(box[1]);
        const num = (v) => String(Number((Number(v) * k).toFixed(6)));
        for (const child of node.children) {
          if (child.type !== 'element') continue;
          const geometry = SCALES[child.name];
          if (geometry && child.attributes.transform === undefined) {
            for (const attr of geometry) {
              if (child.attributes[attr] !== undefined) {
                child.attributes[attr] = num(child.attributes[attr]);
              }
            }
            continue;
          }

          const scale = `scale(${String(Number(k.toFixed(8)))})`;
          child.attributes.transform = child.attributes.transform
            ? `${scale} ${child.attributes.transform}`
            : scale;
        }
        node.attributes.viewBox = '0 0 64 64';
      },
    },
  }),
};

export default {
  multipass: true,
  js2svg: { pretty: true, indent: 2, eol: 'lf', finalNewline: true },
  plugins: [
    scaleToLibraryViewBox,
    // keepDataAttrs would otherwise preserve exporter leftovers like data-name.
    // inlineStyles defaults to `onlyMatchedOnce`, which leaves a <style> block
    // alone as soon as one of its classes styles more than one element — the
    // usual shape of an Illustrator export, and the one thing that kept an
    // icon's fills in CSS where nothing else in this config can read them.
    {
      name: 'preset-default',
      params: {
        overrides: {
          removeUnknownsAndDefaults: { keepDataAttrs: false },
          inlineStyles: { onlyMatchedOnce: false },
        },
      },
    },
    // inlineStyles (inside the preset) lands CSS classes in a style attribute;
    // this turns those into the presentation attributes the rest of the config
    // reads. It has to run after the preset, so multipass does the baking.
    'convertStyleToAttrs',
    // `xml:space="preserve"` is an Illustrator habit. There is no text in the
    // library for it to preserve whitespace in, and svgo's own passes leave it.
    // The separator has to move: the attribute's own name contains a colon.
    { name: 'removeAttrs', params: { elemSeparator: '|', attrs: 'svg|xml:space' } },
    'removeDimensions',
    'sortAttrs',
    removeRootFillNone,
    removeDeadClipRule,
    explicitBackgroundFill,
  ],
};
