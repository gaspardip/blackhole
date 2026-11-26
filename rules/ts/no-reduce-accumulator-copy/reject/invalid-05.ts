declare const items: number[];
items.reduce(function (acc, item) { return Object.assign({}, item, acc); }, {});
