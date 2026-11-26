[1].reduce((acc, n) => (() => [...acc, n])(), []);
[1].reduce((acc, n) => (0, () => Object.assign({}, acc, n))(), {});
