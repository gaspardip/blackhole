const original = client.send; client.send = function (...args) { args[0] = 'changed'; return Reflect.apply(original, this, args); }; client.send = original;
