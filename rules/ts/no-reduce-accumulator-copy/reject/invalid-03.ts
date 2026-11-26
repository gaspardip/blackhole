declare const items: number[];
items.reduce((acc, item, index, array) => Object.assign({}, acc, item), {});
