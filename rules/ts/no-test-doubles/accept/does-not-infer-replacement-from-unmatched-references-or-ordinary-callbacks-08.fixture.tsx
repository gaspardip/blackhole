const original = client.send; client.send = function (...args) { calls.push(args); return Reflect.apply(original, this, args); }; client.send = original;
