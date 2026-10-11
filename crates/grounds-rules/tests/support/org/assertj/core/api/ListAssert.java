package org.assertj.core.api;
public class ListAssert { @CheckReturnValue public ObjectAssert singleElement() { return null; } @CheckReturnValue public ListAssert withFailMessage(String m) { return this; } public ListAssert hasSize(int n) { return this; } }
@interface CheckReturnValue {}
