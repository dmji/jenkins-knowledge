/**
 * Deploy an application to an environment.
 * @example deployApp([environment: 'staging'])
 */
def call(Map config, boolean dryRun = false) {
    helper(config)
}

def helper(Map config) { }

def overloaded(String name) { }
def overloaded(String name, int retries) { }

def methodMissing(String name, args) { delegate.invokeMethod(name, args) }
