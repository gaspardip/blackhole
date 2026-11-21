test("t", async () => { await new Promise(r => self.setTimeout(r,100)); assert(work()); });
