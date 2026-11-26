declare const items: number[];
items.reduce((acc, item) => {
  const next = {};
  return Object.assign(next, acc, item);
}, {});
