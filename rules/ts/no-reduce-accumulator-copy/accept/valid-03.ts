declare const items: number[];
items.reduce((acc, item) => Object.assign(acc, acc, item), {});
