test.each([1, 2])("loads %i", async () => { expect(await load()).toEqual(expected); });
