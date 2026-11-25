const shape = 'shape'; const o = { [(shape)]: 1 };
class C { [((shape as K) satisfies K)!] = 1; }
o[((shape)!)];
