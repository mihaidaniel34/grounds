package mn;
import jakarta.inject.*;
interface Engine {}
@Singleton class V8Engine implements Engine {}
@Singleton class V6Engine implements Engine {}
@Qualifier @interface V8 {}
@Singleton class ByNamedPrefix { @Inject ByNamedPrefix(@Named("v8") Engine e) {} }
@Singleton class ByAnnotationPrefix { @Inject ByAnnotationPrefix(@V8 Engine e) {} }
@Singleton class ByAny { @Inject ByAny(@io.micronaut.context.annotation.Any Engine e) {} }
@Singleton class Typo { @Inject Typo(@Named("v12") Engine e) {} } // expect
