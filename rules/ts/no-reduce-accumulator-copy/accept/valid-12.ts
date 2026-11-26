declare const items: number[];
items.reduce((acc, item) => { const snapshot = () => Object.assign({}, acc); return acc; }, {});
