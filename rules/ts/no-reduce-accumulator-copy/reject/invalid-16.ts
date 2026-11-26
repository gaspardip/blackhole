declare const items: number[];
items.reduce((acc, item) => { const next = Array.from(acc); next.push(item); return next; }, []);
