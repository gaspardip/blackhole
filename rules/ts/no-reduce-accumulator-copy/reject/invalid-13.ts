items.reduce((acc, item) => { const next = acc.slice(); next.push(item); return next; }, []);
