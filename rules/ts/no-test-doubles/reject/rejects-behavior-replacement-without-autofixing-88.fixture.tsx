const create = vi.fn.bind(vi); const bound = create.bind(null, () => 1); bound();
