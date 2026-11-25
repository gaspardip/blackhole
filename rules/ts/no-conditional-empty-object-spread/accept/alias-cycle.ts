const empty = {};
const left = c ? empty : right;
const right = left;
consume({ ...(c ? empty : x) });
