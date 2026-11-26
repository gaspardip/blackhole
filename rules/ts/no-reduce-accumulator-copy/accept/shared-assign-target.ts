const dst = {}; [1].reduce((acc, x) => Object.assign(dst, acc, { [x]: x }), dst);
const alias = dst; items.reduce(acc => Object.assign((alias as T), acc), {});
