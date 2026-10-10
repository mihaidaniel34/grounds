class A {
    private static Object instance;
    private static volatile Object safe;
    static Object get() {
        if (instance == null) { // expect
            synchronized (A.class) {
                if (instance == null) {
                    instance = new Object();
                }
            }
        }
        return instance;
    }
    static Object getSafe() {
        if (safe == null) {
            synchronized (A.class) {
                if (safe == null) safe = new Object();
            }
        }
        return safe;
    }
}
