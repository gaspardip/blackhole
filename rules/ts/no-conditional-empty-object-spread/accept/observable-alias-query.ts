const empty = {};
const unused = empty;
export type T = typeof unused;
consume({ ...(c ? empty : x) });
