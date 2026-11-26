function copy(acc: object, item: object): object;
function copy(acc: object, item: object) {
  return Object.assign({}, acc, item);
}
const reducer = copy;
items.reduce(reducer, {});
