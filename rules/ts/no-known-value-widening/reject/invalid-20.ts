type Command = () => void; const startCommand = () => {}; function outer() { type Open = Record<string, Command>; const commands: Open = { start: startCommand }; }
