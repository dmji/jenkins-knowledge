import groovy.json.JsonOutput
import jenkins.model.Jenkins

final int MAX_ITEMS_PER_REGISTRY = 100
final int MAX_PARAMETERS_PER_STEP = 20
final int MAX_DECLARATIVE_ITEMS = 100
final int MAX_TEXT_LENGTH = 256
final int MAX_OUTPUT_BYTES = 262144

def errors = []
def safeText = { value ->
    if (value == null) {
        return null
    }
    // The repository-owned probe only accepts String results from the fixed
    // Jenkins/JVM accessors. Never invoke arbitrary plugin-object toString().
    if (!(value instanceof String)) {
        return null
    }
    String text = (String) value
    return text.length() > MAX_TEXT_LENGTH ? text.substring(0, MAX_TEXT_LENGTH) : text
}

def result = [
    schemaVersion: '1.0.0',
    templateVersion: '1',
    groovyVersion: null,
    javaVersion: null,
    javaVendor: null,
    jenkinsCoreVersion: null,
    extensionRegistries: [],
    errors: errors
]

try {
    result.groovyVersion = safeText(GroovySystem.getVersion())
} catch (Throwable ignored) {
    errors << [probe: 'groovy-version', code: 'probe-failed']
}

try {
    result.javaVersion = safeText(System.getProperty('java.version'))
} catch (Throwable ignored) {
    errors << [probe: 'java-version', code: 'probe-failed']
}

try {
    result.javaVendor = safeText(System.getProperty('java.vendor'))
} catch (Throwable ignored) {
    errors << [probe: 'java-vendor', code: 'probe-failed']
}

def jenkins = null
try {
    jenkins = Jenkins.getInstanceOrNull()
    if (jenkins == null) {
        errors << [probe: 'jenkins-core-version', code: 'jenkins-unavailable']
    } else {
        result.jenkinsCoreVersion = safeText(Jenkins.VERSION)
    }
} catch (Throwable ignored) {
    errors << [probe: 'jenkins-core-version', code: 'probe-failed']
}

def appendDeclarativeItem = { registry, extension, category, suppliedName, nameKind, contexts, metadata = [:] ->
    if (registry.items.size() >= MAX_DECLARATIVE_ITEMS) {
        registry.truncated = true
        return
    }
    String itemName = safeText(suppliedName)
    if (itemName == null || itemName.trim().isEmpty()) {
        return
    }
    Class describableClass = null
    try {
        describableClass = extension.getClazz()
    } catch (Throwable ignored) {
        // Non-descriptor extension points do not expose a describable class.
    }
    Class implementationClass = describableClass ?: extension.getClass()
    def item = [
        category: category,
        name: itemName,
        nameKind: nameKind,
        className: safeText(implementationClass.getName()),
        descriptorClass: safeText(extension.getClass().getName()),
        contexts: contexts
    ]
    metadata.each { key, value ->
        if (value != null) item[key] = value
    }
    try {
        def displayName = safeText(extension.getDisplayName())
        if (displayName != null) item.displayName = displayName
    } catch (Throwable ignored) {
        // Display metadata is optional.
    }
    if (describableClass != null) {
        try {
            Class modelType = Class.forName(
                'org.jenkinsci.plugins.structs.describable.DescribableModel',
                false,
                jenkins.getPluginManager().getUberClassLoader()
            )
            def model = modelType.getMethod('of', Class).invoke(null, describableClass)
            def parameters = []
            for (def parameter : model.getParameters()) {
                if (parameters.size() >= MAX_PARAMETERS_PER_STEP) {
                    item.parametersTruncated = true
                    break
                }
                def parameterName = safeText(parameter.getName())
                if (parameterName == null || parameterName.trim().isEmpty()) continue
                def parameterRecord = [name: parameterName]
                try {
                    Class parameterType = parameter.getType()
                    if (parameterType != null) parameterRecord.type = safeText(parameterType.getName())
                } catch (Throwable ignored) {}
                try {
                    parameterRecord.required = parameter.isRequired() == true
                } catch (Throwable ignored) {}
                try {
                    def description = safeText(parameter.getDescription())
                    if (description != null) parameterRecord.description = description
                } catch (Throwable ignored) {}
                try {
                    def defaultValue = parameter.getDefaultParameterValue()
                    if (defaultValue instanceof String || defaultValue instanceof Number || defaultValue instanceof Boolean) {
                        parameterRecord.default = defaultValue
                    }
                } catch (Throwable ignored) {}
                parameters << parameterRecord
            }
            item.parameters = parameters
        } catch (Throwable ignored) {
            // A missing/incompatible structs API leaves parameter metadata unknown.
        }
    }
    try {
        def plugin = jenkins.getPluginManager().whichPlugin(implementationClass)
        if (plugin != null) item.plugin = safeText(plugin.getShortName())
    } catch (Throwable ignored) {
        // Plugin attribution is optional.
    }
    registry.items << item
}

def collectDeclarativeExtensions = { registry ->
    def descriptorSpecs = [
        [
            category: 'agent',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.agent.DeclarativeAgentDescriptor',
            contexts: ['pipeline', 'stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'option',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.options.DeclarativeOptionDescriptor',
            contexts: ['pipeline', 'stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'when',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.when.DeclarativeStageConditionalDescriptor',
            contexts: ['stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'post-condition',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.model.BuildCondition',
            contexts: ['pipeline', 'stage'],
            enumeration: 'condition-methods'
        ],
        [
            category: 'trigger',
            className: 'hudson.triggers.Trigger',
            contexts: ['pipeline'],
            enumeration: 'descriptor-list'
        ],
        [
            category: 'parameter',
            className: 'hudson.model.ParameterDefinition',
            contexts: ['pipeline'],
            enumeration: 'descriptor-list'
        ]
    ]
    int discoveredTypes = 0
    descriptorSpecs.each { descriptorSpec ->
        try {
            Class extensionType = Class.forName(
                descriptorSpec.className,
                false,
                jenkins.getPluginManager().getUberClassLoader()
            )
            discoveredTypes++
            def extensions
            if (descriptorSpec.enumeration == 'static-all') {
                extensions = extensionType.getMethod('all').invoke(null)
            } else if (descriptorSpec.enumeration == 'condition-methods') {
                extensions = extensionType.getMethod('getConditionMethods').invoke(null)
            } else {
                extensions = jenkins.getDescriptorList(extensionType)
            }
            if (descriptorSpec.enumeration == 'condition-methods') {
                extensions.each { conditionName, extension ->
                    def description = null
                    try { description = safeText(extension.getDescription()) } catch (Throwable ignored) {}
                    appendDeclarativeItem(registry, extension, descriptorSpec.category, conditionName, 'dsl-token',
                        descriptorSpec.contexts, description == null ? [:] : [displayName: description])
                }
            } else {
                for (def extension : extensions) {
                    String itemName
                    String nameKind = 'dsl-token'
                    try {
                        itemName = extension.getName()
                    } catch (Throwable ignored) {
                        itemName = extension.getClazz().getSimpleName()
                        nameKind = 'implementation-class'
                    }
                    def metadata = [:]
                    if (descriptorSpec.category == 'option') {
                        try { metadata.stageOnly = extension.isStageOnly() == true } catch (Throwable ignored) {}
                        try { metadata.canUseInStage = extension.canUseInStage() == true } catch (Throwable ignored) {}
                        if (metadata.stageOnly == true) {
                            metadata.contexts = ['stage']
                        } else if (metadata.canUseInStage == false) {
                            metadata.contexts = ['pipeline']
                        }
                    }
                    if (descriptorSpec.category == 'when') {
                        try { metadata.allowedChildrenCount = extension.getAllowedChildrenCount() } catch (Throwable ignored) {}
                        try { metadata.invisible = extension.isInvisible() == true } catch (Throwable ignored) {}
                    }
                    appendDeclarativeItem(registry, extension, descriptorSpec.category, itemName, nameKind,
                        metadata.contexts ?: descriptorSpec.contexts, metadata)
                }
            }
        } catch (ClassNotFoundException ignored) {
            errors << [probe: 'declarative-extensions', code: 'extension-type-unavailable']
        } catch (Throwable ignored) {
            errors << [probe: 'declarative-extensions', code: 'enumeration-failed']
        }
    }
    if (discoveredTypes == 0) {
        registry.status = 'unavailable'
        registry.errorCode = 'extension-type-unavailable'
    } else if (errors.any { it.probe == 'declarative-extensions' }) {
        registry.status = 'unknown'
        registry.errorCode = 'enumeration-failed'
    }
}

def registrySpecs = [
    [
        name: 'pipeline-steps',
        className: 'org.jenkinsci.plugins.workflow.steps.StepDescriptor'
    ],
    [
        name: 'pipeline-globals',
        className: 'org.jenkinsci.plugins.workflow.cps.GlobalVariable'
    ],
    [
        name: 'declarative-extensions',
        className: 'org.jenkinsci.plugins.pipeline.modeldefinition.agent.DeclarativeAgentDescriptor'
    ]
]

registrySpecs.each { spec ->
    def registry = [
        name: spec.name,
        status: 'available',
        items: [],
        truncated: false
    ]

    try {
        if (jenkins == null) {
            throw new IllegalStateException('Jenkins instance unavailable')
        }
        if (spec.name == 'declarative-extensions') {
            collectDeclarativeExtensions(registry)
            result.extensionRegistries << registry
            return
        }

        Class extensionType = Class.forName(
            spec.className,
            false,
            jenkins.getPluginManager().getUberClassLoader()
        )
        def extensions = jenkins.getExtensionList(extensionType)
        int count = 0

        for (def extension : extensions) {
            if (count >= MAX_ITEMS_PER_REGISTRY) {
                registry.truncated = true
                break
            }

            String itemName = spec.name == 'pipeline-steps'
                ? extension.getFunctionName()
                : extension.getName()
            if (itemName == null || itemName.trim().isEmpty()) {
                continue
            }

            def item = [
                name: safeText(itemName),
                className: safeText(extension.getClass().getName())
            ]
            if (spec.name == 'pipeline-steps') {
                try {
                    def displayName = safeText(extension.getDisplayName())
                    if (displayName != null) item.displayName = displayName
                } catch (Throwable ignored) {
                    // Display metadata is optional across Jenkins/plugin versions.
                }
                try {
                    Class modelType = Class.forName(
                        'org.jenkinsci.plugins.structs.describable.DescribableModel',
                        false,
                        jenkins.getPluginManager().getUberClassLoader()
                    )
                    def model = modelType.getMethod('of', Class).invoke(null, extension.getClazz())
                    def parameters = []
                    for (def parameter : model.getParameters()) {
                        if (parameters.size() >= MAX_PARAMETERS_PER_STEP) {
                            item.parametersTruncated = true
                            break
                        }
                        def parameterName = safeText(parameter.getName())
                        if (parameterName == null || parameterName.trim().isEmpty()) {
                            continue
                        }
                        def parameterRecord = [name: parameterName]
                        try {
                            Class parameterType = parameter.getType()
                            if (parameterType != null) {
                                parameterRecord.type = safeText(parameterType.getName())
                            }
                        } catch (Throwable ignored) {}
                        try {
                            parameterRecord.required = parameter.isRequired() == true
                        } catch (Throwable ignored) {}
                        try {
                            def description = safeText(parameter.getDescription())
                            if (description != null) parameterRecord.description = description
                        } catch (Throwable ignored) {}
                        try {
                            def defaultValue = parameter.getDefaultParameterValue()
                            if (defaultValue instanceof String || defaultValue instanceof Number || defaultValue instanceof Boolean) {
                                parameterRecord.default = defaultValue
                            }
                        } catch (Throwable ignored) {}
                        parameters << parameterRecord
                    }
                    item.parameters = parameters
                } catch (Throwable ignored) {
                    // structs-plugin metadata is optional; preserve the descriptor itself.
                }
            }
            try {
                def plugin = jenkins.getPluginManager().whichPlugin(extension.getClass())
                if (plugin != null) {
                    item.plugin = safeText(plugin.getShortName())
                }
            } catch (Throwable ignored) {
                // Plugin attribution is optional; do not infer it.
            }

            registry.items << item
            count++
        }
    } catch (ClassNotFoundException ignored) {
        registry.status = 'unavailable'
        registry.errorCode = 'extension-type-unavailable'
        errors << [probe: spec.name, code: 'extension-type-unavailable']
    } catch (Throwable ignored) {
        registry.status = 'unknown'
        registry.errorCode = 'enumeration-failed'
        errors << [probe: spec.name, code: 'enumeration-failed']
    }

    result.extensionRegistries << registry
}

def output = JsonOutput.toJson(result)
if (output.getBytes('UTF-8').length > MAX_OUTPUT_BYTES) {
    result.extensionRegistries.each { registry ->
        if (registry.items.size() > 10) {
            registry.items = registry.items.take(10)
            registry.truncated = true
        }
    }
    errors << [probe: 'output', code: 'output-truncated']
    output = JsonOutput.toJson(result)
}

println(output)
