await expect.poll(() => { return sent.length; }).toBe(2);
