const original = client.send; client.send = (...args) => Reflect.apply(original, this, args); client.send = original;
