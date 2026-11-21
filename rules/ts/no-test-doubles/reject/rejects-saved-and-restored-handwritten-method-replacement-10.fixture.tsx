const original = client.send; client.send = function (...args) { Reflect.apply(original, this, args); return false; }; client.send = original;
