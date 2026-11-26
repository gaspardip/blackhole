items.reduce(acc => {
  const holder = { target: {} };
  const key = 'target';
  holder[key] = shared;
  return Object.assign(holder[key], acc);
}, {});

items.reduce(acc => {
  const holder = { target: {} };
  const key = holder[key];
  return Object.assign(holder[key], acc);
}, {});
