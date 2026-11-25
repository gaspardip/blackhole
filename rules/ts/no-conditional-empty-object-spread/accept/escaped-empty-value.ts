const row = {}; row.id = 1; consume({ ...(c ? row : x) });
const payload = c ? {} : x; payload.id = 1; consume({ ...payload });
export const exposed = {}; consume({ ...(c ? exposed : x) });
const passed = {}; observe(passed); consume({ ...(c ? x : passed) });
const named = {}; const alias = named; observe(alias); consume({ ...(c ? named : x) });
const returned = {}; function f() { return returned; } consume({ ...(c ? returned : x) });
const condition = {}; consume(condition ? x : y, { ...(c ? condition : x) });
let mutable = {}; consume({ ...(c ? mutable : x) });
