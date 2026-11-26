declare const items: number[];
items.reduce((acc, item) => { let alias = acc; alias = item; return Object.assign({}, alias); }, {});
