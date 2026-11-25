enum E { [`shape`] = 1 }
import data from "./data.json" with { "reshape": "shape" };
enum S { "\uD800shape" = 1 }
const o = { ["\uDFFFShape"]: 1 };
class C { [`\uD800reShape`]() {} }
