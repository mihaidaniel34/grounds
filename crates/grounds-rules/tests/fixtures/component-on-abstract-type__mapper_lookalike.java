package lookalike;
import org.springframework.stereotype.Component;

// a project annotation named Mapper is not MapStruct: nothing is generated
@interface Mapper {}
@Mapper @Component abstract class LookalikeMapper {} // expect
