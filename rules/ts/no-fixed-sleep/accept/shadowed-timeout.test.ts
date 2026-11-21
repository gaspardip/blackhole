const setTimeout = invokeImmediately; test('x',async()=>{await new Promise(r=>setTimeout(r,100));assert(work());});
function check(setTimeout: Timer) { test('x',async()=>{await new Promise(r=>setTimeout(r,100));assert(work());}); }
