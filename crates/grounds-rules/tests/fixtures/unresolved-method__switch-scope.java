class SwitchScopeCase {
    String v = "field";
    void accept(int value) {}
    void test(int kind) {
        switch (kind) {
            case 0:
                int v;
            case 1:
                v = 2;
                this.accept(v);
                break;
        }
    }
    void bad() { this.accept("s"); } // expect
}
