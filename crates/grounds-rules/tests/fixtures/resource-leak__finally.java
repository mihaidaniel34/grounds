import java.io.*;
class ResourceCases {
    static void closedInFinally() throws IOException {
        FileInputStream f = new FileInputStream("/dev/null");
        try {
            int b = f.read();
        } finally {
            f.close();
        }
    }
    static void breakRunsFinally() throws IOException {
        FileInputStream f = new FileInputStream("/dev/null");
        while (true) {
            try { break; }
            finally { f.close(); }
        }
    }
    static void leaked() throws IOException {
        FileInputStream f = new FileInputStream("/dev/null"); // expect
        int b = f.read();
    }
}
