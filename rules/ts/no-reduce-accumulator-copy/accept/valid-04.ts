declare const items: number[];
items.reduce((acc, item) => { acc[item.id] = { ...item }; return acc; }, {});
