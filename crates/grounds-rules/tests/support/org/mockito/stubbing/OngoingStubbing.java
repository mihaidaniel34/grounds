package org.mockito.stubbing;
public interface OngoingStubbing<T> { OngoingStubbing<T> thenReturn(T v); OngoingStubbing<T> thenThrow(Throwable t); }
