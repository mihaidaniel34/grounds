// grounds: no-findings
import java.io.*;
class A {
    int m(int n, Reader reader) throws IOException {
        if (n > 0) return n;
        else if ((n = reader.read()) < 0) return -1;
        else if (n > 0) return n;
        return 0;
    }
}
