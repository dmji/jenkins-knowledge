import groovy.json.JsonOutput
import jenkins.model.Jenkins

final int MAX_ITEMS_PER_REGISTRY = 100
final int MAX_PARAMETERS_PER_STEP = 32
final int MAX_REQUIRED_CONTEXT_ITEMS = 32
final int MAX_TEXT_LENGTH = 256
final int MAX_OUTPUT_BYTES = 262144

def errors = []
def safeText = { value ->
    if (!(value instanceof String)) {
        return null
    }
    String text = (String) value
    return text.length() > MAX_TEXT_LENGTH ? text.substring(0, MAX_TEXT_LENGTH) : text
}
def safeHtmlText = { value ->
    if (!(value instanceof String)) {
        return null
    }
    String text = (String) value
    text = text.replaceAll(/(?is)<script\\b[^>]*>.*?<\\/script>/, ' ')
    text = text.replaceAll(/(?is)<style\\b[^>]*>.*?<\\/style>/, ' ')
    text = text.replaceAll(/<[^>]*>/, ' ')
    text = text.replace('&nbsp;', ' ')
    text = text.replaceAll(/\\s+/, ' ').trim()
    return safeText(text)
}

def result = [
    schemaVersion: '1.0.0',
    templateVersion: '2',
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

def registrySpecs = [
    [
        name: 'pipeline-steps',
        className: 'org.jenkinsci.plugins.workflow.steps.StepDescriptor'
    ],
    [
        name: 'pipeline-globals',
        className: 'org.jenkinsci.plugins.workflow.cps.GlobalVariable'
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

            try {
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
                try {
                    def plugin = jenkins.getPluginManager().whichPlugin(extension.getClass())
                    if (plugin != null) {
                        item.plugin = safeText(plugin.getShortName())
                    }
                } catch (Throwable ignored) {
                    // Plugin attribution is optional; do not infer it.
                }

                if (spec.name == 'pipeline-steps') {
                    item.displayName = safeText(extension.getDisplayName())
                    item.implementationClass = safeText(extension.clazz?.getName())
                    item.isAdvanced = extension.isAdvanced() == true
                    item.isMetaStep = extension.isMetaStep() == true
                    item.takesImplicitBlockArgument = extension.takesImplicitBlockArgument() == true

                    def requiredContext = extension.getRequiredContext()
                    if (requiredContext != null) {
                        item.requiredContext = requiredContext
                            .collect { contextType -> contextType instanceof Class ? safeText(contextType.getName()) : null }
                            .findAll { contextName -> contextName != null }
                            .unique()
                            .sort()
                            .take(MAX_REQUIRED_CONTEXT_ITEMS)
                        if (requiredContext.size() > MAX_REQUIRED_CONTEXT_ITEMS) {
                            registry.truncated = true
                        }
                    }

                    def metaStepType = extension.getMetaStepArgumentType()
                    if (metaStepType instanceof Class) {
                        item.metaStepArgumentType = safeText(metaStepType.getName())
                    }

                    try {
                        Class modelClass = Class.forName(
                            'org.jenkinsci.plugins.structs.describable.DescribableModel',
                            false,
                            jenkins.getPluginManager().getUberClassLoader()
                        )
                        def model = modelClass.getMethod('of', Class.class).invoke(null, extension.clazz)
                        item.parametersStatus = 'available'
                        item.parameters = []
                        def modelParameters = model.getParameters()
                        int parameterCount = 0
                        for (def parameter : modelParameters) {
                            if (parameterCount >= MAX_PARAMETERS_PER_STEP) {
                                registry.truncated = true
                                break
                            }
                            def parameterType = parameter.getErasedType()
                            def parameterRecord = [
                                name: safeText(parameter.getName()),
                                type: parameterType instanceof Class ? safeText(parameterType.getName()) : null,
                                required: parameter.isRequired() == true,
                                deprecated: parameter.isDeprecated() == true,
                                defaultValueStatus: 'not-exposed'
                            ]
                            try {
                                def parameterHelp = safeHtmlText(parameter.getHelp())
                                if (parameterHelp != null) {
                                    parameterRecord.description = parameterHelp
                                }
                            } catch (Throwable ignored) {
                                // Parameter help is optional metadata.
                            }
                            if (parameterRecord.name != null) {
                                item.parameters << parameterRecord
                                parameterCount++
                            }
                        }
                        try {
                            def help = safeHtmlText(model.getHelp())
                            if (help != null) {
                                item.description = help
                            }
                        } catch (Throwable ignored) {
                            // Descriptor help is optional metadata.
                        }
                    } catch (ClassNotFoundException ignored) {
                        item.parametersStatus = 'unavailable'
                        errors << [probe: spec.name, code: 'extension-type-unavailable']
                    } catch (Throwable ignored) {
                        item.parametersStatus = 'unknown'
                        errors << [probe: spec.name, code: 'enumeration-failed']
                    }
                } else {
                    // Do not instantiate a global variable value; only enumerate its registered provider.
                    item.implementationClass = item.className
                    item.typeStatus = 'not-exposed'
                }

                registry.items << item
                count++
            } catch (Throwable ignored) {
                errors << [probe: spec.name, code: 'enumeration-failed']
            }
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
        registry.items = registry.items.take(25)
        registry.items.each { item ->
            if (item.parameters instanceof List && item.parameters.size() > 4) {
                item.parameters = item.parameters.take(4)
            }
            if (item.requiredContext instanceof List && item.requiredContext.size() > 8) {
                item.requiredContext = item.requiredContext.take(8)
            }
        }
        registry.truncated = true
    }
    errors << [probe: 'output', code: 'output-truncated']
    output = JsonOutput.toJson(result)
}

println(output)
