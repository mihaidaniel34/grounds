class A {
    void m() {
        while (true) { // expect
            work();
        }
    }
    void n() {
        for (;;) { // expect
            work();
        }
    }
    int o() {
        while (true) {
            if (done()) return 1;
        }
    }
    void p() {
        while (true) {
            if (done()) break;
        }
    }
    void q() {
        while (true) {
            try { Thread.sleep(1); } catch (InterruptedException e) { return; }
        }
    }
    void work() {}
    boolean done() { return true; }
}
class StuckCondition {
    int total;
    void neverUpdated(int n, int[] a) {
        int i = 0;
        while (i < n) { // expect
            total += a[i];
        }
        boolean done = false;
        do { // expect
            total++;
        } while (!done);
    }
    void fine(int n, java.util.Queue<String> q) {
        int i = 0;
        while (i < n) { i++; }
        int j = 0;
        while (j < n) { if (total > 3) break; total++; }
        outer:
        for (int k = 0; k < n; k++) {
            while (j < n) { if (total > 3) continue outer; total++; }
        }
        // other threads and calls can change what these read
        while (!q.isEmpty()) { total++; }
        while (total < n) { work(); }
    }
    void work() { total++; }
}
