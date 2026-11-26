declare const items: number[];
items.reduce((acc, item) => { acc.push(Object.assign({}, item)); return acc; }, []);
