items.reduce(copy, {});
function copy(acc, item) {
  return Object.assign({}, acc, item);
}
const reducer = copy;
items.reduce(reducer, {});
