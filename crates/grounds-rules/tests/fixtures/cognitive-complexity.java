// option max=5
class A {
    int complex(int a, int b) { // expect
        int r = 0;
        if (a > 0) {
            for (int i = 0; i < a; i++) {
                if (i % 2 == 0 && b > 0 || a < 3) {
                    r++;
                } else {
                    r--;
                }
            }
        }
        return r;
    }
    int simple(int a) { return a > 0 ? 1 : 0; }
}
