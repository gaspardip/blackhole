consume({ [(effect(), 'userShape')]: 0 });
const names = { key: 'userShape', 0: 'listShape' };
consume({ [names?.key]: 0, [names?.[0]]: 0 });
