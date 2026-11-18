const original = client.send; function replacement() { return false; } client.send = replacement; client.send = original;
