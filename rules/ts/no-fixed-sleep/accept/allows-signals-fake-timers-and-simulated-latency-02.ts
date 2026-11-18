test('x', async () => { const ready = Promise.withResolvers(); await ready.promise; expect(readState()).toEqual(expectedState); });
