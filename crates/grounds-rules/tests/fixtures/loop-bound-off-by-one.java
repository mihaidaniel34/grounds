import java.util.ArrayList;
import java.util.List;
import java.util.Map;

class A {
    int[] data;

    int sum(int[] a) {
        int s = 0;
        for (int i = 0; i <= a.length; i++) s += a[i]; // expect
        for (int i = 0; i < a.length; i++) s += a[i];
        for (int i = 0; a.length >= i; ++i) { s += a[i]; } // expect
        for (int i = 0; i <= this.data.length; i++) {
            s += this.data[i]; // expect
        }
        return s;
    }

    void lists(List<String> list, String s, StringBuilder sb, Map<Integer, String> m) {
        for (int i = 0; i <= list.size(); i++) {
            System.out.println(list.get(i)); // expect
        }
        for (int i = 0; i <= s.length(); i += 1) {
            System.out.println(s.charAt(i)); // expect
        }
        for (int i = 0; i <= sb.length(); i++) {
            System.out.println(sb.charAt(i)); // expect
        }
        // a map lookup by number is not an index
        for (int i = 0; i <= m.size(); i++) {
            System.out.println(m.get(i));
        }
    }

    void shifted(int[] a, int[] b, List<String> list) {
        // one-based counting
        for (int i = 1; i <= a.length; i++) a[i - 1] = i;
        // another array may be longer
        for (int i = 0; i <= a.length; i++) b[i] = 0;
        // the bound is not the length
        for (int i = 0; i <= a.length - 1; i++) a[i] = 0;
        for (int i = 0; i <= list.size(); i++) System.out.println(list.get(i - 1 < 0 ? 0 : i - 1));
    }

    void guarded(int[] a, String s, List<String> list) {
        // the extra round stands for "end of input"
        for (int i = 0; i <= s.length(); i++) {
            char c = i < s.length() ? s.charAt(i) : '\n';
            System.out.println(c);
        }
        for (int i = 0; i <= a.length; i++) {
            if (i == a.length) break;
            a[i] = 0;
        }
        for (int i = 0; i <= a.length; i++) {
            try {
                a[i] = 0;
            } catch (ArrayIndexOutOfBoundsException e) {
                return;
            }
        }
        // the list grows while the loop runs
        for (int i = 0; i <= list.size(); i++) {
            list.add("x");
            System.out.println(list.get(i));
        }
        // the array is swapped for a bigger one
        for (int i = 0; i <= a.length; i++) {
            a = new int[a.length + 1];
            a[i] = 0;
        }
        // not a step of one
        for (int i = 0; i <= a.length; i += 2) {
            i--;
            a[i] = 0;
        }
    }
}
