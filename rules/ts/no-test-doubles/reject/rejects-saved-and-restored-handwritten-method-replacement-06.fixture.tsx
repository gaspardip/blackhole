const original = client.send; client.send = async function (...args) { return Reflect.apply(original, this, args); }; client.send = original;
