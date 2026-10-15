class A {
    // FIXME this is broken // expect
    void m() { }

    /**
     * Returns the size.
     * FIXME off by one for empty input // expect
     */
    int size() { return 0; }
}
/** Accepts 'xxx.xxx.xxx.xxx' blocks. */
class Placeholders {
    // Phone format placeholder: XXX-XXXX
    // header for C:\\Users\\xxx\\AppData
    // XXX: really broken // expect
    /*
     * Format: XXX-XXX
     * XXX not thread safe // expect
     */
    void f() { }
}
