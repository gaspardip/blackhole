((items as Rows)?.[`reduce`] as Reduce)?.((acc, item) => ((acc as Rows)?.['concat'] as Concat)?.(item), []);
