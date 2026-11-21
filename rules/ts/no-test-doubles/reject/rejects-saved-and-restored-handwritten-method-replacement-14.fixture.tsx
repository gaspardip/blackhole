const original = client.send; client.send = function (...args) { throw new Error('failed'); return Reflect.apply(original, this, args); }; client.send = original;
