let k = "current"; k = next; expect.poll(() => calls[k].length).toBe(1);
