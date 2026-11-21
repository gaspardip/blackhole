const setTimeout = globalThis.setTimeout; test('x',async()=>{await new Promise(r=>setTimeout(r,100));assert(work());});
