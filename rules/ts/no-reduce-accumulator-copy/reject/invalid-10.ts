declare const items: number[];
items.reduce((acc, item) => { const next = Object.assign({}, acc); next[item.id] = item; return next; }, {});
