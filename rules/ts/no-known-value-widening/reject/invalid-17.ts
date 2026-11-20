type Command = () => void; const startCommand = () => {}; type Open = { [key: string]: Command }; const source = { start: startCommand }; const commands: Open = source;
