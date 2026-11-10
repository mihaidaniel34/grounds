// grounds: test, grounds: no-findings
class MatrixTest {
    void fill(long[][] m, int qps, int seconds) {
        for (int i = 0; i < m.length; i++) {
            for (int j = 0; j < m[i].length; j++) {
                m[i][j] = (i + 1) * (j + 1);
            }
        }
        long total = qps * seconds;
    }
}
