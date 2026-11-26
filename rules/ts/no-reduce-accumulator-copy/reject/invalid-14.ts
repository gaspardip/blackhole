declare const items: number[];
items.reduce((acc, item) => { const alias = acc; return alias.concat(item); }, []);
