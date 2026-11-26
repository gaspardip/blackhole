declare const items: number[];
items.reduce(acc => {
  const holder = { target: {} };
  const key = 'target';
  return Object.assign(holder[key], acc);
}, {});

items.reduce(acc => {
  const holder = { target: shared };
  const names = { choice: 'tar' + 'get' };
  const key = names['choice'];
  holder[key] = {};
  return Object.assign(holder[key], acc);
}, {});
