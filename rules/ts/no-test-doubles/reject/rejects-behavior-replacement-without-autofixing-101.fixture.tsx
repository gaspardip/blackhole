const remove = Reflect.deleteProperty.bind(Reflect, window, "fetch"); remove();
