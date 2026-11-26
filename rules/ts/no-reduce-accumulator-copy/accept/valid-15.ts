declare const items: number[];
function run(Object) { return items.reduce((acc, item) => Object.assign({}, acc), {}); }
