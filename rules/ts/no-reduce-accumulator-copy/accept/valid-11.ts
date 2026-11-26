declare const items: number[];
items.reduce((acc, item) => { function copy(acc) { return Object.assign({}, acc); } return acc; }, {});
