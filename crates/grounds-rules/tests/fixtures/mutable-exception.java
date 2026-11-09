class MyException extends Exception {
    private String detail; // expect
    private final int code = 1;
    void setDetail(String detail) { this.detail = detail; }
}
// written by the constructor only, no setter: nothing can change it later
class SetOnce extends RuntimeException {
    private int status;
    SetOnce(int status) { this.status = status; }
    int getStatus() { return status; }
}
@lombok.Data
class WithSetter extends RuntimeException {
    private int status; // expect
}
@lombok.Value
class ValueException extends RuntimeException { String code; }
