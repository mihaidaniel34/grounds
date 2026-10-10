class Outer {
    int field;
    class Uses {
        int get() { return field; }
    }
    class Independent { // expect
        int x;
        int get() { return x * 2; }
    }
    class CallsOuter {
        void run() { work(); }
    }
    static class AlreadyStatic { }
    void work() {}
}
