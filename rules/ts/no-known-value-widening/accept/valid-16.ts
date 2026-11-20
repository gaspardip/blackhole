type Command = () => void; const startCommand = () => {}; type PermissionLevels = { readonly [Level in Permission]: number }; const levels: PermissionLevels = { admin: 1 };
