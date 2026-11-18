const { prototype: reader } = FileReader; const method = `readAsDataURL`; Reflect.set(reader, method, replacement);
