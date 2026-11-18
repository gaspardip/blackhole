const original = client.send; client.send = function (...args) { Reflect.set(args, 0, 'changed'); return Reflect.apply(original, this, args); }; client.send = original;
