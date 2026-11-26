declare const items: number[];
const initial = []; items.reduce((acc, item) => acc.concat(item), initial);
