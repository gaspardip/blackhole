type Command = () => void; const startCommand = () => {}; type Commands = Record<string, Command>; const commands = { start: startCommand } as const satisfies Commands;
