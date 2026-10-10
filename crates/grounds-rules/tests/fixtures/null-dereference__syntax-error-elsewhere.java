// an unfinished declaration later in the file must not hide findings in
// well-formed code
class Test {
    void f() { String s = null; s.length(); } // expect

    void unfinished(
