[1].reduce(acc => (() => [...acc]).call(null), []);
[1].reduce(acc => Reflect.apply(() => [...acc], null, []), []);
[1].reduce(acc => (() => [...acc]).bind(null)(), []);
