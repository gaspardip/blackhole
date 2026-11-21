const original = client.send; client.send = function (...args) { return Reflect.apply(original, other, args); }; client.send = original;
