declare const items: number[];
items.reduce((acc, item) => acc.toSpliced(acc.length, 0, item), []);
