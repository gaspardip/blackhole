const original = client.send; client.send = function (...args) { args.reverse(); return Reflect.apply(original, this, args); }; client.send = original;
