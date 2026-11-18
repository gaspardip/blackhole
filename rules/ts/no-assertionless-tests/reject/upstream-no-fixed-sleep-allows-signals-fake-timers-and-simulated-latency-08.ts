test('x', async () => { await new Promise((resolve) => { emitter.once('done', resolve); }); });
