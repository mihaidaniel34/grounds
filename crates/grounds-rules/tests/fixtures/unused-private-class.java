class A {
    private static class Unused { } // expect
    private static class Used { }
    private enum E { X } // expect
    Used u = new Used();
}
