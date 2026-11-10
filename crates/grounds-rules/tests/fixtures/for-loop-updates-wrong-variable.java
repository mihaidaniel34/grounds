class A {
    void m(int n) {
        for (int i = 0; i < n; i++) {
            for (int j = 0; j < n; i++) { } // expect
        }
        for (int k = 0; k < n; k++) { }
    }
}
class Iterators {
    int[] indexed(java.util.List<Integer> values) {
        int[] out = new int[values.size()];
        int i = 0;
        // an iterator loop that keeps a separate index: next() advances the iterator
        for (java.util.Iterator<Integer> it = values.iterator(); it.hasNext(); i++) {
            out[i] = it.next();
        }
        return out;
    }
    void copyPaste(java.util.List<Integer> values, int n) {
        int k = 0;
        for (int j = 0; j < n; k++) { values.add(j); } // expect
    }
}
class IteratorNotAdvanced {
    int count(java.util.List<Integer> values) {
        int i = 0;
        // hasNext() alone does not advance the iterator
        for (java.util.Iterator<Integer> it = values.iterator(); it.hasNext(); i++) { // expect
            if (i > 10) break;
        }
        return i;
    }
}
class IteratorOnlyObserved {
    void m(java.util.List<String> xs) {
        int j = 0;
        // nextIndex() reads the position without moving it
        for (java.util.ListIterator<String> it = xs.listIterator(); it.hasNext(); j++) { // expect
            System.out.println(it.nextIndex());
        }
    }
}
class ScannerOnlyObserved {
    void m(java.io.File file) throws java.io.IOException {
        int count = 0;
        // Scanner's hasNextXxx queries read ahead without consuming
        for (java.util.Scanner sc = new java.util.Scanner(file); sc.hasNextDouble(); count++) { // expect
            if (sc.hasNextDouble()) System.out.println(count);
        }
        for (java.util.Scanner sc = new java.util.Scanner(file); sc.hasNextDouble(); count++) {
            System.out.println(sc.nextDouble());
        }
    }
}
class OwnCursor {
    interface Cursor { boolean hasNextAndAdvance(); }
    void m(Cursor c) {
        int seen = 0;
        // not a Scanner: a hasNext... method of another type may move it
        for (Cursor cur = c; cur.hasNextAndAdvance(); seen++) {
            if (cur.hasNextAndAdvance()) System.out.println(seen);
        }
    }
}
