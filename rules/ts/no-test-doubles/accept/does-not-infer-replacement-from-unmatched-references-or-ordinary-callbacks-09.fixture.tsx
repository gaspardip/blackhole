const original = client.send; client.send = function (...args) { return Reflect.apply(original, this, args); }; client.send = original;
