declare const items: number[];
items.reduce(acc => Object.assign(([] as T), acc), []);
