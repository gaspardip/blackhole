const original = client.send; client.send = function (...args) { Reflect.apply(original, this, args); throw new Error('failed'); }; client.send = original;
