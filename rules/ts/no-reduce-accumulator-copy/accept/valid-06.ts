declare const items: number[];
items.reduce((acc, item) => { acc.push(item.slice()); return acc; }, []);
