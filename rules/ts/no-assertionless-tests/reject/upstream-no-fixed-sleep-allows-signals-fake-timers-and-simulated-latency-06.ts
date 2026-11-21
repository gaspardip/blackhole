test('x', async () => { await Promise.race([work, new Promise((resolve) => { setTimeout(() => resolve('pending'), 0); })]); });
