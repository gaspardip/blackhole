test("loads", { tags: ["slow"] }, async () => { expect(await load()).toEqual(expected); });
