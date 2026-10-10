# Rule catalog

312 rules. `grounds explain <id>` prints the full description.

Tier `edit` rules run after every edit; all rules run at checkpoints. Rules marked *off* are disabled by default (enable in `grounds.toml`).

## bad-practice (38)

| id | severity | tier | origin |
|---|---|---|---|
| `default-charset` | info | checkpoint | spotbugs:DM_DEFAULT_ENCODING, errorprone:DefaultCharset, sonar:S1943 |
| `case-conversion-without-locale` *off* | info | checkpoint | spotbugs:DM_CONVERT_CASE, errorprone:StringCaseLocaleUsage, sonar:S1449 |
| `empty-catch-block` | warning | checkpoint | sonar:S108, sonar:S2486, pmd:EmptyCatchBlock, spotbugs:DE_MIGHT_IGNORE, intellij:EmptyCatchBlock |
| `catch-throwable` | warning | checkpoint | sonar:S1181, pmd:AvoidCatchingThrowable, intellij:CatchGenericClass |
| `catch-null-pointer` | warning | checkpoint | sonar:S1696, pmd:AvoidCatchingNPE |
| `catch-generic-exception` *off* | info | checkpoint | pmd:AvoidCatchingGenericException, sonar:S2221 |
| `throw-generic-exception` | info | checkpoint | sonar:S112, pmd:AvoidThrowingRawExceptionTypes |
| `lost-stack-trace` | warning | checkpoint | sonar:S1166, pmd:PreserveStackTrace, errorprone:UnusedException |
| `log-and-rethrow` | info | checkpoint | sonar:S2139 |
| `print-stack-trace` | warning | checkpoint | sonar:S1148, sonar:S4507, pmd:AvoidPrintStackTrace, intellij:CallToPrintStackTrace |
| `finalize-usage` | warning | checkpoint | sonar:S1113, sonar:S1111, pmd:AvoidCallingFinalize, spotbugs:FI_EXPLICIT_INVOCATION |
| `close-in-finally-unguarded` | info | checkpoint | sonar:S2093, pmd:UseTryWithResources |
| `exception-message-only-logged` | info | checkpoint | sonar:S1166, intellij:ThrowableNotPassedToLogger |
| `catch-return-null` | info | checkpoint | sonar:S1168, pmd:ReturnEmptyCollectionRatherThanNull |
| `ignored-status-result` | warning | checkpoint | sonar:S899, spotbugs:RV_RETURN_VALUE_IGNORED_BAD_PRACTICE |
| `collection-method-returns-null` | warning | checkpoint | sonar:S1168, spotbugs:PZLA_PREFER_ZERO_LENGTH_ARRAYS, pmd:ReturnEmptyCollectionRatherThanNull |
| `system-exit` | warning | checkpoint | sonar:S1147, spotbugs:DM_EXIT, pmd:DoNotTerminateVM |
| `system-out-println` | info | checkpoint | sonar:S106, pmd:SystemPrintln, intellij:UseOfSystemOutOrSystemErr |
| `deprecated-api-usage` | info | checkpoint | sonar:S1874, sonar:S5738, intellij:Deprecation |
| `optional-as-parameter-or-field` | info | checkpoint | sonar:S3553, intellij:OptionalUsedAsFieldOrParameterType |
| `loop-counter-modified-in-body` | info | checkpoint | sonar:S127, pmd:AvoidReassigningLoopVariables, intellij:AssignmentToForLoopParameter |
| `public-static-non-final-field` | warning | checkpoint | sonar:S1444, spotbugs:MS_SHOULD_BE_FINAL, pmd:MutableStaticState |
| `public-static-mutable-container` | warning | checkpoint | sonar:S2386, spotbugs:MS_MUTABLE_ARRAY, spotbugs:MS_MUTABLE_COLLECTION, spotbugs:MS_MUTABLE_COLLECTION_PKGPROTECT |
| `static-field-written-from-instance-method` | warning | checkpoint | sonar:S2696, spotbugs:ST_WRITE_TO_STATIC_FROM_INSTANCE_METHOD |
| `field-hides-superclass-field` | info | checkpoint | sonar:S2387, spotbugs:MF_CLASS_MASKS_FIELD, intellij:FieldNameHidesFieldInSuperclass |
| `class-shadows-supertype-name` | warning | checkpoint | sonar:S2176, spotbugs:NM_SAME_SIMPLE_NAME_AS_SUPERCLASS, spotbugs:NM_SAME_SIMPLE_NAME_AS_INTERFACE |
| `missing-serial-version-uid` *off* | info | checkpoint | sonar:S2057, spotbugs:SE_NO_SERIALVERSIONID, pmd:MissingSerialVersionUID |
| `cloneable-without-clone` | info | checkpoint | sonar:S2157, spotbugs:CN_IDIOM |
| `compareto-without-equals` | info | checkpoint | sonar:S1210, spotbugs:EQ_COMPARETO_USE_OBJECT_EQUALS |
| `mutable-enum-field` | warning | checkpoint | sonar:S3066, spotbugs:ME_MUTABLE_ENUM_FIELD, spotbugs:ME_ENUM_FIELD_SETTER |
| `mutable-exception` | info | checkpoint | sonar:S1165 |
| `unused-local-variable` | warning | edit | sonar:S1481, sonar:S1854, pmd:UnusedLocalVariable, intellij:UnusedDeclaration |
| `unused-private-field` | warning | checkpoint | sonar:S1068, spotbugs:URF_UNREAD_FIELD, spotbugs:UUF_UNUSED_FIELD, pmd:UnusedPrivateField |
| `unused-private-method` | warning | checkpoint | sonar:S1144, spotbugs:UPM_UNCALLED_PRIVATE_METHOD, pmd:UnusedPrivateMethod |
| `unused-parameter` | info | checkpoint | sonar:S1172, pmd:UnusedFormalParameter, intellij:UnusedParameters |
| `unused-private-class` | info | checkpoint | sonar:S1144, pmd:UnusedPrivateClass |
| `dead-store` | warning | checkpoint | sonar:S1854, spotbugs:DLS_DEAD_LOCAL_STORE, pmd:UnusedAssignment, intellij:UnusedAssignment |
| `legacy-date-api` *off* | info | checkpoint | sonar:S2143 |

## bug (109)

| id | severity | tier | origin |
|---|---|---|---|
| `string-reference-equality` | warning | edit | sonar:S4973, spotbugs:ES_COMPARING_STRINGS_WITH_EQ, pmd:UseEqualsToCompareStrings, errorprone:StringEquality, intellij:StringEquality |
| `boxed-reference-equality` | warning | edit | sonar:S4973, spotbugs:RC_REF_COMPARISON, errorprone:BoxedPrimitiveEquality, intellij:NumberEquality |
| `equals-incompatible-types` | error | edit | sonar:S2159, spotbugs:EC_UNRELATED_TYPES, errorprone:EqualsIncompatibleType, intellij:EqualsBetweenInconvertibleTypes |
| `array-equals` | error | edit | spotbugs:EC_BAD_ARRAY_COMPARE, errorprone:ArrayEquals, sonar:S2159 |
| `array-tostring-hashcode` | warning | edit | sonar:S2116, spotbugs:DMI_INVOKING_TOSTRING_ON_ARRAY, spotbugs:DMI_INVOKING_HASHCODE_ON_ARRAY, errorprone:ArrayToString, errorprone:ArrayHashCode |
| `collection-incompatible-type` | error | edit | sonar:S2175, spotbugs:GC_UNRELATED_TYPES, errorprone:CollectionIncompatibleType, intellij:SuspiciousMethodCalls |
| `identical-operands` | warning | edit | sonar:S1764, spotbugs:SA_FIELD_SELF_COMPARISON, spotbugs:SA_LOCAL_SELF_COMPARISON, errorprone:SelfEquals, intellij:EqualsWithItself |
| `self-assignment` | error | edit | sonar:S1656, spotbugs:SA_LOCAL_SELF_ASSIGNMENT, spotbugs:SA_FIELD_SELF_ASSIGNMENT, errorprone:SelfAssignment, intellij:SillyAssignment |
| `float-equality` | warning | checkpoint | sonar:S1244, spotbugs:FE_FLOATING_POINT_EQUALITY, intellij:FloatingPointEquality |
| `nan-comparison` | error | edit | spotbugs:FE_TEST_IF_EQUAL_TO_NOT_A_NUMBER, errorprone:EqualsNaN |
| `equals-null` | error | edit | spotbugs:EC_NULL_ARG, pmd:EqualsNull |
| `compareto-specific-value` | error | edit | sonar:S2200, spotbugs:RV_CHECK_COMPARETO_FOR_SPECIFIC_RETURN_VALUE |
| `negated-compareto` | warning | checkpoint | spotbugs:RV_NEGATING_RESULT_OF_COMPARETO |
| `equals-without-hashcode` | error | edit | sonar:S1206, spotbugs:HE_EQUALS_NO_HASHCODE, spotbugs:HE_HASHCODE_NO_EQUALS, pmd:OverrideBothEqualsAndHashcode, intellij:EqualsAndHashcode |
| `equals-overload` | error | edit | sonar:S1201, spotbugs:EQ_SELF_NO_OBJECT, errorprone:NonOverridingEquals, intellij:CovariantEquals |
| `class-name-comparison` | warning | checkpoint | sonar:S1872, spotbugs:EQ_COMPARING_CLASS_NAMES |
| `getclass-on-class` | error | edit | errorprone:GetClassOnClass |
| `bigdecimal-equals` | warning | checkpoint | errorprone:BigDecimalEquals, sonar:S2111 |
| `optional-null-comparison` | warning | edit | sonar:S2789, spotbugs:NP_OPTIONAL_RETURN_NULL, errorprone:OptionalEquality |
| `reference-equality-value-type` | info | checkpoint | errorprone:ReferenceEquality, pmd:CompareObjectsWithEquals, intellij:ObjectEquality |
| `compareto-min-value` | warning | checkpoint | sonar:S2167, spotbugs:CO_COMPARETO_RESULTS_MIN_VALUE |
| `instanceof-always-false` | error | checkpoint | sonar:S2589, spotbugs:BC_IMPOSSIBLE_INSTANCEOF |
| `bad-shift-amount` | error | edit | sonar:S2183, spotbugs:ICAST_BAD_SHIFT_AMOUNT, errorprone:BadShiftAmount |
| `integer-division-in-floating-context` | warning | edit | sonar:S2184, spotbugs:ICAST_IDIV_CAST_TO_DOUBLE, errorprone:IntegerDivision |
| `int-overflow-in-long-context` | warning | checkpoint | sonar:S2184, spotbugs:ICAST_INTEGER_MULTIPLY_CAST_TO_LONG, errorprone:IntLongMath |
| `constant-overflow` | error | edit | errorprone:ConstantOverflow, sonar:S2184 |
| `abs-of-hashcode-or-random` | warning | checkpoint | sonar:S2676, spotbugs:RV_ABSOLUTE_VALUE_OF_HASHCODE, spotbugs:RV_ABSOLUTE_VALUE_OF_RANDOM_INT, errorprone:MathAbsoluteNegative |
| `random-cast-to-int` | error | edit | spotbugs:RV_01_TO_INT |
| `bigdecimal-from-double` | warning | edit | sonar:S2111, spotbugs:DMI_BIGDECIMAL_CONSTRUCTED_FROM_DOUBLE, pmd:AvoidDecimalLiteralsInBigDecimalConstructor |
| `odd-check-remainder` | warning | edit | sonar:S2197, spotbugs:IM_BAD_CHECK_FOR_ODD |
| `comparison-out-of-range` | error | edit | spotbugs:INT_BAD_COMPARISON_WITH_SIGNED_BYTE, spotbugs:INT_BAD_COMPARISON_WITH_INT_VALUE, errorprone:ComparisonOutOfRange |
| `narrowing-compound-assignment` | warning | checkpoint | errorprone:NarrowingCompoundAssignment, sonar:S2164 |
| `overwritten-increment` | error | edit | spotbugs:DLS_OVERWRITTEN_INCREMENT, sonar:S2123 |
| `reversed-assignment-operator` | warning | edit | sonar:S2757 |
| `octal-literal` | warning | checkpoint | sonar:S1314, pmd:AvoidUsingOctalValues |
| `rounding-of-integer-division` | error | edit | spotbugs:ICAST_INT_CAST_TO_DOUBLE_PASSED_TO_CEIL, spotbugs:ICAST_INT_CAST_TO_FLOAT_PASSED_TO_ROUND, sonar:S2184 |
| `char-addition-in-concat` | warning | checkpoint | errorprone:CharacterGetNumericValue, sonar:S2185 |
| `remainder-by-one` | error | checkpoint | spotbugs:INT_BAD_REM_BY_1 |
| `masked-value-comparison` | error | checkpoint | spotbugs:BIT_AND, spotbugs:BIT_AND_ZZ, spotbugs:BIT_IOR |
| `min-max-clamp-swapped` | error | checkpoint | errorprone:BadMinMax, sonar:S3518 |
| `useless-sign-check` | error | checkpoint | sonar:S3981, spotbugs:INT_VACUOUS_COMPARISON, errorprone:ComparisonOutOfRange |
| `floating-cast-in-integer-division` | warning | checkpoint | errorprone:FloatCast, spotbugs:ICAST_INT_2_LONG_AS_INSTANT |
| `indexof-positive-check` | warning | checkpoint | sonar:S2692 |
| `regex-metachar-literal` | error | edit | spotbugs:RE_POSSIBLE_UNINTENDED_PATTERN, errorprone:StringSplitter |
| `invalid-regex` | error | edit | sonar:S5856, errorprone:InvalidPatternSyntax |
| `file-separator-as-regex` | error | checkpoint | spotbugs:RE_CANT_USE_FILE_SEPARATOR_AS_REGULAR_EXPRESSION |
| `stringbuilder-char-constructor` | error | edit | sonar:S1317, pmd:StringBufferInstantiationWithChar, spotbugs:DM_STRING_BUFFER_CHAR |
| `format-string-mismatch` | error | edit | sonar:S2275, sonar:S3457, spotbugs:VA_FORMAT_STRING_MISSING_ARGUMENT, spotbugs:VA_FORMAT_STRING_EXTRA_ARGUMENTS_PASSED, spotbugs:VA_FORMAT_STRING_BAD_CONVERSION, errorprone:FormatString |
| `log-placeholder-mismatch` | warning | edit | sonar:S2275, sonar:S3457, pmd:InvalidLogMessageFormat, errorprone:Slf4jFormatShouldBeConst |
| `abrupt-finally` | warning | checkpoint | sonar:S1143, pmd:ReturnFromFinallyBlock, pmd:DoNotThrowExceptionInFinally, errorprone:Finally |
| `exception-not-thrown` | error | edit | sonar:S3984, spotbugs:RV_EXCEPTION_NOT_THROWN, errorprone:DeadException |
| `iterator-next-no-such-element` | warning | checkpoint | sonar:S2272, spotbugs:IT_NO_SUCH_ELEMENT |
| `hasnext-calls-next` | error | checkpoint | sonar:S1849, spotbugs:DMI_CALLING_NEXT_FROM_HASNEXT |
| `throw-null` | error | checkpoint | sonar:S1696, errorprone:ThrowNull |
| `ignored-return-value` | error | edit | sonar:S2201, spotbugs:RV_RETURN_VALUE_IGNORED, errorprone:ReturnValueIgnored, errorprone:CheckReturnValue, intellij:ResultOfMethodCallIgnored |
| `ignored-read-result` | warning | checkpoint | sonar:S2674, spotbugs:RR_NOT_CHECKED, spotbugs:SR_NOT_CHECKED |
| `stream-not-consumed` | error | edit | sonar:S3958, errorprone:StreamResourceLeak |
| `boolean-method-returns-null` | warning | checkpoint | sonar:S2447, spotbugs:NP_BOOLEAN_RETURN_NULL |
| `tostring-returns-null` | error | checkpoint | sonar:S2225, spotbugs:NP_TOSTRING_COULD_RETURN_NULL, spotbugs:NP_CLONE_COULD_RETURN_NULL |
| `collection-contains-itself` | error | checkpoint | sonar:S2114, spotbugs:DMI_COLLECTIONS_SHOULD_NOT_CONTAIN_THEMSELVES, spotbugs:DMI_USING_REMOVEALL_TO_CLEAR_COLLECTION |
| `arrays-aslist-primitive-array` | error | edit | errorprone:ArraysAsListPrimitiveArray, sonar:S3878 |
| `toarray-downcast` | error | edit | spotbugs:BC_IMPOSSIBLE_DOWNCAST_OF_TOARRAY, sonar:S3878 |
| `collection-modified-during-iteration` | error | checkpoint | errorprone:ModifyCollectionInEnhancedForLoop, sonar:S6466, spotbugs:DMI_ENTRY_SETS_MAY_REUSE_ENTRY_OBJECTS |
| `immutable-collection-modified` | error | edit | sonar:S5838, errorprone:DoNotCall, intellij:ModifyingImmutableCollection |
| `list-remove-int-ambiguity` | warning | checkpoint | errorprone:CollectionUndefinedEquality, sonar:S3878 |
| `empty-control-statement-body` | warning | edit | sonar:S1116, errorprone:EmptyIf, pmd:EmptyControlStatement, intellij:ControlFlowStatementWithoutBraces |
| `identical-branches` | warning | checkpoint | sonar:S1871, sonar:S3923, spotbugs:DB_DUPLICATE_BRANCHES, spotbugs:DB_DUPLICATE_SWITCH_CLAUSES, intellij:IfStatementWithIdenticalBranches |
| `identical-conditions` | error | edit | sonar:S1862, intellij:DuplicateCondition |
| `assignment-in-condition` | warning | edit | sonar:S1121, spotbugs:QBA_QUESTIONABLE_BOOLEAN_ASSIGNMENT, pmd:AssignmentInOperand, intellij:AssignmentUsedAsCondition |
| `non-short-circuit-logic` | warning | checkpoint | sonar:S2178, spotbugs:NS_DANGEROUS_NON_SHORT_CIRCUIT, spotbugs:NS_NON_SHORT_CIRCUIT |
| `null-check-wrong-operator` | error | edit | sonar:S1697, spotbugs:NP_NULL_ON_SOME_PATH, intellij:ConstantConditions |
| `loop-wrong-direction` | error | edit | sonar:S2251, errorprone:LoopDirection |
| `infinite-loop` | warning | checkpoint | sonar:S2189, spotbugs:IL_INFINITE_LOOP |
| `loop-executes-at-most-once` | warning | checkpoint | sonar:S1751, intellij:LoopStatementThatDoesntLoop |
| `switch-fallthrough` | warning | edit | sonar:S128, spotbugs:SF_SWITCH_FALLTHROUGH, errorprone:FallThrough, pmd:ImplicitSwitchFallThrough |
| `assert-with-side-effect` | warning | checkpoint | sonar:S3346, spotbugs:SA_LOCAL_SELF_COMPARISON, intellij:AssertWithSideEffects |
| `constant-boolean-condition` | warning | checkpoint | sonar:S2583, sonar:S2589, intellij:ConstantConditions |
| `for-loop-updates-wrong-variable` | error | edit | sonar:S1994, intellij:ForLoopThatDoesntUseLoopVariable |
| `constructor-calls-overridable-method` | warning | checkpoint | sonar:S1699, pmd:ConstructorCallsOverridableMethod, spotbugs:MC_OVERRIDABLE_METHOD_CALL_IN_CONSTRUCTOR |
| `method-named-like-class` | warning | checkpoint | sonar:S1223, spotbugs:NM_METHOD_CONSTRUCTOR_CONFUSION, pmd:MethodWithSameNameAsEnclosingClass |
| `object-method-name-typo` | error | edit | sonar:S1221, spotbugs:NM_LCASE_HASHCODE, spotbugs:NM_LCASE_TOSTRING, spotbugs:NM_BAD_EQUAL, pmd:SuspiciousHashcodeMethodName |
| `serial-version-uid-modifiers` | warning | checkpoint | sonar:S2057, spotbugs:SE_NONSTATIC_SERIALVERSIONID, spotbugs:SE_NONFINAL_SERIALVERSIONID, spotbugs:SE_NONLONG_SERIALVERSIONID |
| `non-serializable-field` | warning | checkpoint | sonar:S1948, spotbugs:SE_BAD_FIELD |
| `serialization-method-signature` | error | checkpoint | sonar:S2061, spotbugs:SE_METHOD_MUST_BE_PRIVATE, spotbugs:SE_READ_RESOLVE_IS_STATIC, spotbugs:SE_READ_RESOLVE_MUST_RETURN_OBJECT |
| `clone-without-super-clone` | warning | checkpoint | sonar:S1182, spotbugs:CN_IDIOM_NO_SUPER_CALL, pmd:ProperCloneImplementation |
| `accessor-uses-wrong-field` | warning | edit | sonar:S4275 |
| `record-array-component` | warning | checkpoint | sonar:S6218 |
| `equals-without-type-check` | warning | checkpoint | sonar:S2097, spotbugs:BC_EQUALS_METHOD_SHOULD_WORK_FOR_ALL_OBJECTS |
| `null-dereference` | error | edit | sonar:S2259, spotbugs:NP_ALWAYS_NULL, spotbugs:NP_NULL_ON_SOME_PATH, spotbugs:NP_GUARANTEED_DEREF, errorprone:NullAway, intellij:DataFlowIssue |
| `redundant-null-check` | warning | checkpoint | sonar:S2583, sonar:S2589, spotbugs:RCN_REDUNDANT_NULLCHECK_OF_NONNULL_VALUE, spotbugs:RCN_REDUNDANT_NULLCHECK_WOULD_HAVE_BEEN_A_NPE, spotbugs:RCN_REDUNDANT_NULLCHECK_OF_NULL_VALUE, intellij:ConstantConditions |
| `resource-leak` | warning | checkpoint | sonar:S2095, spotbugs:OBL_UNSATISFIED_OBLIGATION, spotbugs:OS_OPEN_STREAM, pmd:CloseResource, errorprone:StreamResourceLeak |
| `optional-get-without-check` | warning | checkpoint | sonar:S3655, intellij:OptionalGetWithoutIsPresent, errorprone:OptionalNotPresent |
| `infinite-recursion` | error | edit | sonar:S2190, spotbugs:IL_INFINITE_RECURSIVE_LOOP, errorprone:InfiniteRecursion |
| `use-after-close` | warning | checkpoint | sonar:S2095, spotbugs:OBL_UNSATISFIED_OBLIGATION_EXCEPTION_EDGE |
| `uninitialized-field-read` | error | checkpoint | spotbugs:UR_UNINIT_READ, sonar:S3077 |
| `null-passed-to-nonnull-parameter` | error | edit | spotbugs:NP_NONNULL_PARAM_VIOLATION, errorprone:NullAway, intellij:ConstantConditions |
| `nonnull-method-returns-null` | error | checkpoint | spotbugs:NP_NONNULL_RETURN_VIOLATION, errorprone:NullAway |
| `week-year-in-date-pattern` | error | edit | sonar:S3986, errorprone:MisusedWeekYear, spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO |
| `day-of-year-in-date-pattern` | error | edit | errorprone:MisusedDayOfYear, spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO |
| `twelve-hour-without-am-pm` | warning | checkpoint | errorprone:MisusedHourOfDay, spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO |
| `minute-month-confusion` | error | edit | errorprone:MisusedDateFormat, sonar:S3986 |
| `invalid-date-value` | error | edit | sonar:S2110, spotbugs:DMI_BAD_MONTH |
| `instant-unsupported-unit` | error | edit | errorprone:JavaInstantGetSecondsGetNano, errorprone:JavaTimeDefaultTimeZone |
| `invalid-time-zone-id` | warning | checkpoint | errorprone:InvalidTimeZoneID, errorprone:InvalidZoneId |
| `unresolved-method` | error | edit | javac:cant.resolve.location.args, intellij:UnresolvedReference |
| `unresolved-constructor` | error | edit | javac:cant.apply.symbol, intellij:UnresolvedReference |
| `unresolved-field` | error | edit | javac:cant.resolve.location, intellij:UnresolvedReference |
| `override-matches-nothing` | error | edit | javac:method.does.not.override.superclass, intellij:MethodDoesntOverride |
| `unimplemented-abstract-method` | error | edit | javac:does.not.override.abstract, intellij:ClassNotImplementingAbstractMethods |

## complexity (5)

| id | severity | tier | origin |
|---|---|---|---|
| `too-many-parameters` | info | checkpoint | sonar:S107, pmd:ExcessiveParameterList |
| `cognitive-complexity` | info | checkpoint | sonar:S3776 |
| `deep-nesting` | info | checkpoint | sonar:S134, pmd:AvoidDeeplyNestedIfStmts |
| `method-too-long` | info | checkpoint | sonar:S138, pmd:NcssCount |
| `complex-boolean-expression` | info | checkpoint | sonar:S1067 |

## concurrency (23)

| id | severity | tier | origin |
|---|---|---|---|
| `interrupted-exception-swallowed` | warning | checkpoint | sonar:S2142 |
| `thread-run-called-directly` | error | edit | sonar:S1217, spotbugs:RU_INVOKE_RUN, pmd:DontCallThreadRun |
| `wait-not-in-loop` | warning | checkpoint | sonar:S2274, spotbugs:WA_NOT_IN_LOOP, pmd:AvoidWaitOutsideLoop |
| `notify-instead-of-notifyall` | info | checkpoint | sonar:S2446, spotbugs:NO_NOTIFY_NOT_NOTIFYALL, pmd:UseNotifyAllInsteadOfNotify |
| `wait-notify-without-lock` | error | checkpoint | sonar:S2273, spotbugs:MWN_MISMATCHED_WAIT, spotbugs:MWN_MISMATCHED_NOTIFY |
| `synchronized-on-value-object` | warning | checkpoint | sonar:S1860, spotbugs:DL_SYNCHRONIZATION_ON_BOXED_PRIMITIVE, spotbugs:DL_SYNCHRONIZATION_ON_SHARED_CONSTANT, spotbugs:DL_SYNCHRONIZATION_ON_BOOLEAN |
| `synchronized-on-getclass` | warning | checkpoint | sonar:S3067, spotbugs:WL_USING_GETCLASS_RATHER_THAN_CLASS_LITERAL |
| `synchronized-on-non-final-field` | warning | checkpoint | sonar:S2445, spotbugs:ML_SYNC_ON_FIELD_TO_GUARD_CHANGING_THAT_FIELD |
| `double-checked-locking` | error | checkpoint | sonar:S2168, spotbugs:DC_DOUBLECHECK, pmd:DoubleCheckedLocking |
| `static-non-thread-safe-field` | warning | checkpoint | sonar:S2885, spotbugs:STCAL_STATIC_SIMPLE_DATE_FORMAT_INSTANCE, spotbugs:STCAL_STATIC_CALENDAR_INSTANCE, pmd:UnsynchronizedStaticFormatter |
| `sleep-while-holding-lock` | warning | checkpoint | sonar:S2276, spotbugs:SWL_SLEEP_WITH_LOCK_HELD |
| `thread-started-in-constructor` | warning | checkpoint | spotbugs:SC_START_IN_CTOR, sonar:S2693 |
| `condition-wait-instead-of-await` | error | edit | spotbugs:DM_MONITOR_WAIT_ON_CONDITION, errorprone:WaitNotInLoop |
| `lock-without-unlock-in-finally` | warning | checkpoint | sonar:S2222, spotbugs:UL_UNRELEASED_LOCK, spotbugs:UL_UNRELEASED_LOCK_EXCEPTION_PATH |
| `concurrent-map-check-then-act` | warning | checkpoint | spotbugs:AT_OPERATION_SEQUENCE_ON_CONCURRENT_ABSTRACTION, sonar:S3824 |
| `volatile-increment` | warning | checkpoint | sonar:S3078, spotbugs:VO_VOLATILE_INCREMENT |
| `synchronized-on-lock` | error | checkpoint | spotbugs:JLM_JSR166_LOCK_MONITORENTER, spotbugs:JLM_JSR166_UTILCONCURRENT_MONITORENTER, errorprone:SynchronizeOnNonFinalField |
| `volatile-array-or-collection` | info | checkpoint | sonar:S3077, spotbugs:VO_VOLATILE_REFERENCE_TO_ARRAY |
| `threadlocal-not-removed` | info | checkpoint | sonar:S5164 |
| `executor-not-shutdown` | warning | checkpoint | sonar:S2095, errorprone:ThreadPriorityCheck |
| `thread-group-usage` | info | checkpoint | sonar:S3014 |
| `synchronized-on-local` | warning | checkpoint | spotbugs:ML_SYNC_ON_UPDATED_FIELD, sonar:S2445 |
| `singleton-mutable-state` | warning | checkpoint | sonar:S3749, intellij:SpringBeanMutableState |

## framework (20)

| id | severity | tier | origin |
|---|---|---|---|
| `bean-not-found` | error | edit | intellij:SpringJavaInjectionPointsAutowiringInspection, spring:NoSuchBeanDefinitionException |
| `ambiguous-bean` | error | edit | intellij:SpringJavaInjectionPointsAutowiringInspection, spring:NoUniqueBeanDefinitionException |
| `qualifier-not-found` | error | edit | intellij:SpringQualifierInjectionInspection, spring:NoSuchBeanDefinitionException |
| `lombok-constructor-drops-qualifier` | warning | edit | lombok:copyableAnnotations, intellij:SpringJavaInjectionPointsAutowiringInspection |
| `field-injection` | info | checkpoint | sonar:S6813, intellij:SpringJavaAutowiredFieldsWarningInspection |
| `inject-static-field` | error | edit | intellij:SpringJavaStaticMembersAutowiringInspection, sonar:S3749 |
| `inject-final-field` | error | edit | intellij:SpringJavaInjectionPointsAutowiringInspection |
| `multiple-inject-constructors` | error | checkpoint | sonar:S6818, intellij:SpringJavaConstructorAutowiringInspection |
| `proxy-annotation-on-non-proxyable-method` | warning | edit | sonar:S2230, sonar:S6814, intellij:SpringTransactionalMethodCallsInspection |
| `self-invocation-bypasses-proxy` | warning | checkpoint | sonar:S6809, intellij:SpringTransactionalMethodCallsInspection |
| `async-method-return-type` | warning | checkpoint | sonar:S5772, intellij:SpringAsyncMethodInspection |
| `scheduled-method-with-parameters` | error | edit | intellij:SpringScheduledMethodInspection |
| `component-on-abstract-type` | warning | checkpoint | intellij:SpringJavaComponentInspection |
| `bean-method-call-in-lite-mode` | warning | checkpoint | intellij:SpringConfigurationProxyMethods, sonar:S6838 |
| `path-variable-mismatch` | error | edit | sonar:S6856, intellij:MVCPathVariableInspection |
| `circular-constructor-dependency` | error | checkpoint | spring:BeanCurrentlyInCreationException, intellij:SpringJavaInjectionPointsAutowiringInspection |
| `invalid-value-placeholder` | error | edit | intellij:SpringPlaceholdersInspection, sonar:S6857 |
| `bean-name-conflict` | error | checkpoint | spring:ConflictingBeanDefinitionException, intellij:SpringJavaBeanNameConflict |
| `final-configuration-class` | error | edit | intellij:SpringFacetCodeInspection, spring:BeanDefinitionParsingException |
| `injection-in-non-bean` | warning | checkpoint | intellij:SpringJavaAutowiredMembersInspection, sonar:S3749 |

## performance (23)

| id | severity | tier | origin |
|---|---|---|---|
| `url-equals-hashcode` | warning | checkpoint | sonar:S2112, spotbugs:DMI_BLOCKING_METHODS_ON_URL, spotbugs:DMI_COLLECTION_OF_URLS, errorprone:URLEqualsHashCode |
| `random-used-once` | warning | checkpoint | sonar:S2119, spotbugs:DMI_RANDOM_USED_ONLY_ONCE |
| `string-constructor` | info | checkpoint | sonar:S2129, spotbugs:DM_STRING_CTOR, spotbugs:DM_STRING_VOID_CTOR, pmd:StringInstantiation |
| `boxed-primitive-constructor` | warning | checkpoint | sonar:S2129, spotbugs:DM_NUMBER_CTOR, spotbugs:DM_BOOLEAN_CTOR, pmd:PrimitiveWrapperInstantiation |
| `replaceall-with-literal` | info | checkpoint | sonar:S5361, pmd:UseStringReplace |
| `case-insensitive-equals` | info | checkpoint | sonar:S1157, pmd:UnnecessaryCaseChange |
| `string-concat-in-loop` | info | checkpoint | sonar:S1643, pmd:UseStringBufferForStringAppends, intellij:StringConcatenationInLoops |
| `log-string-concatenation` | info | checkpoint | sonar:S2629, pmd:GuardLogStatement, intellij:LoggingStringTemplateAsArgument |
| `stringbuilder-append-concat` | info | checkpoint | pmd:InefficientStringBuffering, intellij:StringConcatenationInsideStringBufferAppend |
| `boxed-tostring` | info | checkpoint | sonar:S2131, spotbugs:DM_BOXED_PRIMITIVE_TOSTRING, spotbugs:DM_BOXED_PRIMITIVE_FOR_COMPARE |
| `valueof-to-primitive` | info | checkpoint | sonar:S2130, spotbugs:DM_BOXED_PRIMITIVE_FOR_PARSING |
| `unnecessary-boxing` | info | checkpoint | sonar:S2153, pmd:UnnecessaryBoxing, spotbugs:BX_BOXING_IMMEDIATELY_UNBOXED |
| `regex-compiled-in-loop` | info | checkpoint | sonar:S4248, pmd:AvoidRecompilingPatterns |
| `legacy-synchronized-collection` | info | checkpoint | sonar:S1149, pmd:ReplaceVectorWithList, pmd:ReplaceHashtableWithMap, pmd:AvoidStringBufferField |
| `map-keyset-then-get` | info | checkpoint | sonar:S2864, spotbugs:WMI_WRONG_MAP_ITERATOR |
| `explicit-gc` | warning | checkpoint | sonar:S1215, spotbugs:DM_GC, pmd:DoNotCallGarbageCollectionExplicitly |
| `optional-orelse-eager-call` | info | checkpoint | sonar:S3655, intellij:OptionalGetWithoutIsPresent |
| `new-date-gettime` | info | checkpoint | pmd:AvoidCallingNewDateGetTime, sonar:S2143 |
| `enum-keyed-hash-collection` | info | checkpoint | sonar:S1640, pmd:UseEnumMap |
| `stream-sorted-findfirst` | info | checkpoint | sonar:S4266, intellij:SimplifyStreamApiCallChains |
| `toarray-presized` *off* | info | checkpoint | intellij:ToArrayCallWithZeroLengthArrayArgument, pmd:OptimizableToArrayCall |
| `stream-count-comparison` | info | checkpoint | sonar:S4034 |
| `inner-class-could-be-static` | info | checkpoint | sonar:S2694, spotbugs:SIC_INNER_SHOULD_BE_STATIC, intellij:InnerClassMayBeStatic |

## security (22)

| id | severity | tier | origin |
|---|---|---|---|
| `reflection-set-accessible` *off* | info | checkpoint | sonar:S3011 |
| `hardcoded-credentials` | warning | checkpoint | sonar:S2068, sonar:S6418, spotbugs:DMI_CONSTANT_DB_PASSWORD, spotbugs:HARD_CODE_PASSWORD |
| `weak-hash-algorithm` | warning | checkpoint | sonar:S4790, sonar:S2070, spotbugs:WEAK_MESSAGE_DIGEST_MD5, spotbugs:WEAK_MESSAGE_DIGEST_SHA1 |
| `weak-cipher-algorithm` | error | checkpoint | sonar:S5547, spotbugs:DES_USAGE, spotbugs:TDES_USAGE, spotbugs:CIPHER_INTEGRITY |
| `insecure-cipher-mode` | error | checkpoint | sonar:S5542, spotbugs:ECB_MODE, spotbugs:PADDING_ORACLE, spotbugs:RSA_NO_PADDING |
| `weak-tls-protocol` | error | checkpoint | sonar:S4423, spotbugs:SSL_CONTEXT |
| `trust-all-certificates` | error | checkpoint | sonar:S4830, spotbugs:WEAK_TRUST_MANAGER |
| `hostname-verifier-accepts-all` | error | checkpoint | sonar:S5527, spotbugs:WEAK_HOSTNAME_VERIFIER |
| `predictable-secure-random` | error | checkpoint | sonar:S4347, spotbugs:PREDICTABLE_RANDOM |
| `sql-injection` | warning | checkpoint | sonar:S2077, sonar:S3649, spotbugs:SQL_NONCONSTANT_STRING_PASSED_TO_EXECUTE, spotbugs:SQL_PREPARED_STATEMENT_GENERATED_FROM_NONCONSTANT_STRING, spotbugs:SQL_INJECTION_JDBC |
| `command-injection` | warning | checkpoint | sonar:S2076, sonar:S4721, spotbugs:COMMAND_INJECTION |
| `xml-external-entities` | warning | checkpoint | sonar:S2755, spotbugs:XXE_DOCUMENT, spotbugs:XXE_SAXPARSER, spotbugs:XXE_XMLSTREAMREADER |
| `csrf-protection-disabled` | warning | checkpoint | sonar:S4502, spotbugs:SPRING_CSRF_PROTECTION_DISABLED |
| `permissive-cors` | warning | checkpoint | sonar:S5122, spotbugs:PERMISSIVE_CORS |
| `weak-key-size` | error | checkpoint | sonar:S4426, spotbugs:RSA_KEY_SIZE, spotbugs:BLOWFISH_KEY_SIZE |
| `insecure-cookie` | warning | checkpoint | sonar:S2092, sonar:S3330, spotbugs:INSECURE_COOKIE, spotbugs:HTTPONLY_COOKIE |
| `unsafe-deserialization` *off* | warning | checkpoint | sonar:S5135, spotbugs:OBJECT_DESERIALIZATION |
| `static-initialization-vector` | error | checkpoint | sonar:S3329, spotbugs:STATIC_IV |
| `world-writable-permissions` | warning | checkpoint | sonar:S2612 |
| `insecure-temp-file` | info | checkpoint | sonar:S5443, spotbugs:PATH_TRAVERSAL_IN |
| `cleartext-http-url` *off* | info | checkpoint | sonar:S5332 |
| `request-mapping-without-method` | info | checkpoint | sonar:S3752 |

## style (58)

| id | severity | tier | origin |
|---|---|---|---|
| `literal-first-in-comparison` | info | checkpoint | sonar:S1132, pmd:LiteralsFirstInComparisons, intellij:LiteralAsArgToStringEquals |
| `enum-compared-with-equals` | info | checkpoint | sonar:S4551, pmd:CompareObjectsWithEquals |
| `lowercase-long-literal` | info | checkpoint | sonar:S818, pmd:LongVariable, errorprone:LongLiteralLowerCaseSuffix |
| `string-tostring` | info | checkpoint | sonar:S1858, pmd:StringToString |
| `use-isempty` | info | checkpoint | sonar:S1155, sonar:S7158, pmd:UseCollectionIsEmpty, intellij:SizeReplaceableByIsEmpty |
| `equals-empty-string` | info | checkpoint | pmd:UseStringIsEmpty, sonar:S7158, intellij:StringEqualsEmptyString |
| `useless-string-valueof` | info | checkpoint | sonar:S1153, pmd:UselessStringValueOf |
| `trim-isempty` | info | checkpoint | sonar:S7158, intellij:StringOperationCanBeSimplified |
| `format-without-arguments` | info | checkpoint | sonar:S3457 |
| `redundant-substring` | info | checkpoint | sonar:S4635, intellij:SubstringZero |
| `useless-catch-rethrow` | info | checkpoint | sonar:S2737, pmd:AvoidRethrowingException |
| `catch-instanceof-dispatch` | info | checkpoint | sonar:S1193 |
| `stream-find-is-present` | info | checkpoint | sonar:S4034, intellij:SimplifyStreamApiCallChains |
| `stream-foreach-on-collection` | info | checkpoint | sonar:S1612, intellij:SimplifyStreamApiCallChains |
| `collectors-tolist` *off* | info | checkpoint | sonar:S6204 |
| `map-contains-then-put` | info | checkpoint | sonar:S3824 |
| `charset-forname-constant` | info | checkpoint | sonar:S4719 |
| `optional-ispresent-then-get` | info | checkpoint | sonar:S3655, intellij:OptionalIsPresent |
| `empty-block` | info | checkpoint | sonar:S108, pmd:EmptyControlStatement, intellij:EmptyStatementBody |
| `boolean-literal-comparison` | info | checkpoint | sonar:S1125, pmd:SimplifyBooleanExpressions, intellij:PointlessBooleanExpression |
| `redundant-if-return-boolean` | info | checkpoint | sonar:S1126, pmd:SimplifyBooleanReturns, intellij:SimplifiableIfStatement |
| `double-negation` | info | checkpoint | sonar:S2761, intellij:DoubleNegation |
| `switch-missing-default` *off* | info | checkpoint | sonar:S131, pmd:SwitchStmtsShouldHaveDefault, intellij:SwitchStatementWithoutDefaultBranch |
| `switch-with-few-cases` *off* | info | checkpoint | sonar:S1301, pmd:TooFewBranchesForSwitch |
| `nested-ternary` | info | checkpoint | sonar:S3358, intellij:NestedConditionalExpression |
| `collapsible-if` | info | checkpoint | sonar:S1066, pmd:CollapsibleIfStatements, intellij:IfStatementMissingBreakInLoop |
| `unused-label` | info | checkpoint | sonar:S1065, intellij:UnusedLabel |
| `redundant-jump` | info | checkpoint | sonar:S3626, pmd:UnnecessaryReturn, intellij:UnnecessaryReturnStatement |
| `local-immediately-returned` | info | checkpoint | sonar:S1488, pmd:UnnecessaryLocalBeforeReturn, intellij:UnnecessaryLocalVariable |
| `redundant-cast` | info | checkpoint | sonar:S1905, pmd:UnnecessaryCast, intellij:RedundantCast |
| `inverted-comparison` | info | checkpoint | sonar:S1940, pmd:LogicInversion, intellij:NegatedEqualityExpression |
| `stray-semicolon` | info | checkpoint | sonar:S1116, pmd:UnnecessarySemicolon, intellij:UnnecessarySemicolon |
| `else-after-return` *off* | info | checkpoint | intellij:RedundantElse, pmd:UnnecessaryElse |
| `utility-class-instantiable` | info | checkpoint | sonar:S1118, pmd:UseUtilityClass, intellij:UtilityClassWithoutPrivateConstructor |
| `constant-interface` | info | checkpoint | sonar:S1214, pmd:ConstantsInInterface |
| `class-naming` | info | checkpoint | sonar:S101, pmd:ClassNamingConventions |
| `method-naming` | info | checkpoint | sonar:S100, pmd:MethodNamingConventions |
| `constant-naming` | info | checkpoint | sonar:S115, pmd:FieldNamingConventions |
| `field-naming` | info | checkpoint | sonar:S116, pmd:FieldNamingConventions |
| `local-variable-naming` | info | checkpoint | sonar:S117, pmd:LocalVariableNamingConventions, pmd:FormalParameterNamingConventions |
| `package-naming` | info | checkpoint | sonar:S120, pmd:PackageCase |
| `type-parameter-naming` *off* | info | checkpoint | sonar:S119, pmd:GenericsNaming |
| `abstract-class-public-constructor` | info | checkpoint | sonar:S5993 |
| `static-member-via-instance` | info | checkpoint | sonar:S2209, pmd:StaticMemberAccessedViaInstance, intellij:StaticMethodReferencedViaSubclass |
| `empty-method` | info | checkpoint | sonar:S1186, pmd:UncommentedEmptyMethodBody |
| `missing-override` | info | checkpoint | sonar:S1161, pmd:MissingOverride, intellij:MissingOverrideAnnotation |
| `useless-override` | info | checkpoint | sonar:S1185, pmd:UselessOverridingMethod |
| `redundant-modifier` | info | checkpoint | sonar:S2333, pmd:UnnecessaryModifier, intellij:UnnecessaryInterfaceModifier |
| `exception-class-naming` | info | checkpoint | sonar:S2166, pmd:ClassNamingConventions |
| `abstract-class-without-abstract-methods` *off* | info | checkpoint | sonar:S1694, pmd:AbstractClassWithoutAbstractMethod |
| `private-constructor-class-not-final` | info | checkpoint | pmd:ClassWithOnlyPrivateConstructorsShouldBeFinal, sonar:S2974 |
| `unused-import` | info | edit | sonar:S1128, pmd:UnnecessaryImport, intellij:UnusedImport |
| `unused-pattern-variable` | info | checkpoint | sonar:S1481, intellij:UnusedDeclaration |
| `junit5-unnecessary-public` *off* | info | checkpoint | sonar:S5786 |
| `todo-comment` | info | checkpoint | sonar:S1135, pmd:TodoComment |
| `fixme-comment` | warning | checkpoint | sonar:S1134 |
| `commented-out-code` | info | checkpoint | sonar:S125 |
| `redundant-response-body` | info | checkpoint | sonar:S6837 |

## testing (14)

| id | severity | tier | origin |
|---|---|---|---|
| `assertion-argument-order` | warning | checkpoint | sonar:S3415, pmd:JUnitAssertionsShouldIncludeMessage |
| `test-without-assertion` | warning | checkpoint | sonar:S2699, pmd:UnitTestShouldIncludeAssert |
| `junit5-private-test-method` | error | checkpoint | sonar:S5810 |
| `mixed-junit-versions` | error | checkpoint | sonar:S5967, sonar:S6073 |
| `assert-true-with-comparison` | info | checkpoint | sonar:S5785, pmd:SimplifiableTestAssertion |
| `assertion-compares-same-value` | error | checkpoint | sonar:S5863 |
| `thread-sleep-in-test` | info | checkpoint | sonar:S2925 |
| `assertj-assertion-without-check` | error | checkpoint | sonar:S2970, errorprone:AssertJIsNull |
| `mockito-incomplete-stubbing` | error | checkpoint | errorprone:MockitoUsage, sonar:S6068 |
| `test-class-without-tests` | info | checkpoint | sonar:S2187 |
| `junit5-static-nested-class` | warning | checkpoint | sonar:S5790 |
| `disabled-test-without-reason` | info | checkpoint | sonar:S1607 |
| `constant-assertion` | warning | checkpoint | sonar:S2701, sonar:S5845 |
| `assertthrows-multiple-calls` | info | checkpoint | sonar:S5778 |

