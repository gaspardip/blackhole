declare const items: number[];
items.reduce((acc, item) => { const alias = acc; return Object.assign({}, alias, item); }, {});
