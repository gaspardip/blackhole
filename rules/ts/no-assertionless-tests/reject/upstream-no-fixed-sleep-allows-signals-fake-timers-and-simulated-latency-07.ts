test('x', async () => { await new Promise((resolve) => { setTimeout(resolve, 10); queueMicrotask(resolve); }); });
