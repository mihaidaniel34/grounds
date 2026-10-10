// option max=3
class A {
    void m(int a) {
        if (a > 0) {
            for (int i = 0; i < a; i++) {
                while (a > 1) {
                    if (a > 2) { // expect
                        a--;
                    }
                }
            }
        }
    }
}
