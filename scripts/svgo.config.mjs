// House style for icons/cryptoicon and icons/fiaticon (see scripts/normalize-icons.sh).
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

export default {
  multipass: true,
  js2svg: { pretty: true, indent: 2, eol: 'lf', finalNewline: true },
  plugins: [
    'preset-default',
    'removeDimensions',
    'sortAttrs',
    removeRootFillNone,
    removeDeadClipRule,
    explicitBackgroundFill,
  ],
};
