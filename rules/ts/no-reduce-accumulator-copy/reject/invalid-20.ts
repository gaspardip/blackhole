declare const items: number[];
items.reduce((acc, item) => acc.with(0, item), []);
