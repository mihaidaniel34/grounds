class A {
    int m() {
        try {
            return 1;
        } finally {
            return 2; // expect
        }
    }
    void n() {
        try { } finally {
            throw new IllegalStateException(); // expect
        }
    }
    void o() {
        for (int i = 0; i < 3; i++) {
            try { } finally {
                for (int j = 0; j < 2; j++) { if (j == 1) break; }
                continue; // expect
            }
        }
    }
    void p() {
        try { } finally {
            try { throw new Exception(); } catch (Exception e) { }
        }
    }
}
