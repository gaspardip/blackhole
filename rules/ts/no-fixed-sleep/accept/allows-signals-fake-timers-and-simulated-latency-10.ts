test('x', async () => { const timer = setTimeout(cancel, 10); await work(); clearTimeout(timer); expect(readState()).toEqual(expectedState); });
